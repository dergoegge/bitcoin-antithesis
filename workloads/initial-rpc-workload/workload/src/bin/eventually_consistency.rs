use std::env;
use std::thread;
use std::time::{Duration, Instant};

use bitcoin::{Address, Network, WitnessProgram, WitnessVersion};
use bitcoin_antithesis_workload::{
    create_client, disconnect_blocked_by_pruning, download_blocked_by_pruning, find_fork_height,
    get_all_nodes, get_blockchain_info, get_chain_tips, set_network_active, BlockchainInfo, Client,
};

/// Time the nodes get to follow a freshly mined block before the next snapshot.
const CONVERGENCE_WAIT: Duration = Duration::from_secs(1);

/// Default of [`retry_budget`].
const DEFAULT_RETRY_BUDGET: Duration = Duration::from_secs(60 * 60);

/// Total time spent driving the cluster onto a single chain before it is
/// declared inconsistent.
///
/// A driver that outlives the test run is stopped before it asserts anything at
/// all, which leaves the property unchecked rather than failed, so the budget can
/// be matched to the length of the run it is used in.
fn retry_budget() -> Duration {
    match env::var("EVENTUALLY_RETRY_BUDGET_SECS") {
        Ok(value) => match value.parse() {
            Ok(secs) => Duration::from_secs(secs),
            Err(e) => {
                eprintln!("Ignoring EVENTUALLY_RETRY_BUDGET_SECS={}: {}", value, e);
                DEFAULT_RETRY_BUDGET
            }
        },
        Err(_) => DEFAULT_RETRY_BUDGET,
    }
}

/// One node's chain state within a snapshot.
struct NodeState {
    /// Position of the node in the client list.
    index: usize,
    name: String,
    info: BlockchainInfo,
}

/// What one snapshot of the cluster says about convergence.
struct Round {
    /// Number of nodes that were asked, so that a node missing from the snapshot
    /// is visible.
    expected: usize,
    states: Vec<NodeState>,
    /// Nodes that can never reach the most-work chain because of pruning.
    blocked_by_pruning: Vec<serde_json::Value>,
    /// Whether the nodes that are able to converge share a tip / a height.
    hashes_converged: bool,
    heights_converged: bool,
    /// Node whose chain to extend to drive the cluster forward, as a position in
    /// `states`.
    extend: Option<usize>,
}

impl Round {
    /// Whether every node answered, i.e. the snapshot covers the whole cluster.
    fn complete(&self) -> bool {
        self.states.len() == self.expected
    }

    fn fully_converged(&self) -> bool {
        self.hashes_converged && self.heights_converged
    }

    fn same_height_block_race(&self) -> bool {
        self.heights_converged && !self.hashes_converged
    }

    /// Whether every node that answered is on `tip`, so that the only thing left
    /// to wait for is a node coming back rather than a chain being followed.
    fn reachable_at(&self, tip: Option<&str>) -> bool {
        let Some(tip) = tip else {
            return false;
        };
        !self.states.is_empty() && self.states.iter().all(|s| s.info.bestblockhash == tip)
    }

    /// Whether the whole cluster is on `tip`, the block the driver mined last.
    /// Nothing is excused: a node that pruning stranded on another chain isn't on
    /// it either.
    fn all_at(&self, tip: Option<&str>) -> bool {
        self.complete() && self.reachable_at(tip)
    }

    fn block_hashes(&self) -> Vec<(&str, &str)> {
        self.states
            .iter()
            .map(|s| (s.name.as_str(), s.info.bestblockhash.as_str()))
            .collect()
    }

    fn block_heights(&self) -> Vec<(&str, u64)> {
        self.states
            .iter()
            .map(|s| (s.name.as_str(), s.info.blocks))
            .collect()
    }

    fn prune_heights(&self) -> Vec<(&str, u64)> {
        self.states
            .iter()
            .filter_map(|s| s.info.pruneheight.map(|h| (s.name.as_str(), h)))
            .collect()
    }
}

fn main() {
    let nodes = get_all_nodes();

    let mut clients: Vec<(String, Client)> = Vec::new();
    for (i, node_config) in nodes.iter().enumerate() {
        let name = format!("node{}", i + 1);
        match create_client(node_config) {
            Ok(c) => clients.push((name, c)),
            Err(e) => eprintln!("{} client creation failed: {}", name, e),
        }
    }

    // Force every node to drop and re-establish its connections, so that a
    // healed network fault doesn't leave the cluster split on dead peers.
    for active in [false, true] {
        for (name, client) in clients.iter() {
            if let Err(e) = set_network_active(client, active) {
                eprintln!("{} setnetworkactive {} failed: {}", name, active, e);
            }
        }
        thread::sleep(Duration::from_secs(5));
    }

    // Snapshot the cluster and, until every node is on the block the driver mined
    // last, extend one node's chain so that the rest of the cluster has a
    // most-work chain to follow, then give them a moment to follow it. Waiting
    // for a block of the driver's own rather than for any shared tip means the
    // nodes have to prove they still follow the chain, not just that they agree
    // on an old one.
    let address = mining_address();
    let budget = retry_budget();
    let start = Instant::now();
    let mut mined_tip: Option<String> = None;
    let mut rounds: u64 = 0;
    let mut last_log = String::new();
    let round = loop {
        rounds += 1;
        let round = take_round(&clients, nodes.len(), rounds, &mut last_log);

        if round.all_at(mined_tip.as_deref()) {
            println!(
                "All nodes are on the block mined last after {} round(s)",
                rounds
            );
            break round;
        }
        if start.elapsed() >= budget {
            println!(
                "Giving up on convergence after {} round(s) in {}s",
                rounds,
                start.elapsed().as_secs()
            );
            break round;
        }

        // Nothing is mined while every node that did answer is already on the
        // block mined last, because then there is no chain left to be followed,
        // only an unreachable node to wait for.
        if !round.reachable_at(mined_tip.as_deref()) {
            if let Some(state) = round.extend.map(|position| &round.states[position]) {
                let (name, client) = &clients[state.index];
                // Overtake every chain the cluster knows about in one go, so that
                // a node that is hundreds of blocks behind doesn't need hundreds
                // of rounds to get ahead. Once it is the highest chain, each round
                // adds a single block to it.
                let height = state.info.blocks;
                let target = highest_known_height(&clients).max(height);
                let blocks = (target + 1 - height).max(1);
                if let Some(tip) = extend_chain(name, client, &address, blocks) {
                    mined_tip = Some(tip);
                }
            }
        }

        thread::sleep(CONVERGENCE_WAIT);
    };

    assert_convergence(&round, rounds, start.elapsed(), mined_tip.as_deref());
}

/// Take a snapshot of every node and judge how far the cluster is from
/// converging on it.
///
/// The snapshot is only logged when it differs from `last_log`, so that waiting
/// for a slow node doesn't bury the run's log in identical rounds.
fn take_round(
    clients: &[(String, Client)],
    expected: usize,
    round: u64,
    last_log: &mut String,
) -> Round {
    let mut states: Vec<NodeState> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for (index, (name, client)) in clients.iter().enumerate() {
        match get_blockchain_info(client) {
            Ok(info) => {
                lines.push(format!(
                    "  {}: height={} tip={} chainwork={} pruneheight={:?}",
                    name, info.blocks, info.bestblockhash, info.chainwork, info.pruneheight
                ));
                states.push(NodeState {
                    index,
                    name: name.clone(),
                    info,
                });
            }
            Err(e) => lines.push(format!("  {}: getblockchaininfo failed: {}", name, e)),
        }
    }

    let log = lines.join("\n");
    if log != *last_log {
        println!("Round {}:\n{}", round, log);
        *last_log = log;
    }

    let complete = states.len() == expected;
    let mut blocked_by_pruning: Vec<serde_json::Value> = Vec::new();
    let mut hashes_converged = false;
    let mut heights_converged = false;
    let mut extend = None;

    // Pruning can make convergence onto the most-work chain impossible in two
    // ways, from either side of the fork:
    //
    //   - the lagging node pruned the undo data of a block it has to disconnect,
    //     so it can never reorg away from its own chain
    //   - every node on the most-work chain pruned the blocks above the fork
    //     point, so the lagging node can never download that branch
    //
    // Both are pruning limitations rather than consistency violations. The way
    // out of either is to extend the stranded node's own chain until it is the
    // most-work chain, so the rest of the cluster follows it instead. If that
    // fails too, the node is excluded from the convergence check below.
    //
    // The nodes that didn't answer are simply left out: whatever is reachable
    // still gets driven onto one chain, and the snapshot being incomplete is
    // reported on its own below.
    if let Some((best_position, best)) = states
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.info.chainwork.cmp(&b.info.chainwork))
    {
        let best_client = &clients[best.index].1;
        let best_tip = best.info.bestblockhash.clone();

        // The only nodes that hold the most-work chain's block data.
        let best_chain: Vec<&BlockchainInfo> = states
            .iter()
            .map(|state| &state.info)
            .filter(|info| info.bestblockhash == best_tip)
            .collect();

        // Of the stranded nodes, the one closest to outpacing the most-work
        // chain, i.e. the cheapest chain for the cluster to be pushed onto.
        let mut stranded: Option<usize> = None;

        for (position, state) in states.iter().enumerate() {
            if state.info.bestblockhash == best_tip {
                continue;
            }
            let info = &state.info;
            let client = &clients[state.index].1;
            // The fork point is at or below both tips, so the search only has to
            // cover the heights that both nodes have a block at.
            let fork_height =
                find_fork_height(client, best_client, info.blocks.min(best.info.blocks));
            let disconnect_blocked =
                fork_height.is_some_and(|height| disconnect_blocked_by_pruning(info, height));
            let download_blocked =
                fork_height.is_some_and(|height| download_blocked_by_pruning(height, &best_chain));

            let details = serde_json::json!({
                "node": state.name,
                "fork_height": fork_height,
                "pruneheight": info.pruneheight,
                "height": info.blocks,
                "tip": info.bestblockhash,
                "best_tip": best_tip,
                "best_height": best.info.blocks,
                "best_chain_prune_heights": best_chain
                    .iter()
                    .map(|info| info.pruneheight)
                    .collect::<Vec<Option<u64>>>(),
                "disconnect_blocked": disconnect_blocked,
                "download_blocked": download_blocked,
            });

            // Only a snapshot of the whole cluster proves that nobody could
            // have served the missing blocks, so a partial one isn't reported.
            if complete {
                antithesis_sdk::assert_sometimes!(
                    disconnect_blocked,
                    "A pruned node can't reorg onto the most-work chain because the fork point is below its pruneheight",
                    &details
                );

                antithesis_sdk::assert_sometimes!(
                    download_blocked,
                    "A node can't sync onto the most-work chain because every node on it has pruned the blocks above the fork point",
                    &details
                );
            }

            if disconnect_blocked || download_blocked {
                blocked_by_pruning.push(details);
                let outpaces = |current: usize| info.chainwork > states[current].info.chainwork;
                if stranded.is_none_or(outpaces) {
                    stranded = Some(position);
                }
            }
        }

        // Extending a stranded node's chain is the only thing that can still
        // resolve a pruning deadlock; otherwise the most-work chain is extended
        // so that the nodes behind it get a fresh tip to chase.
        extend = Some(stranded.unwrap_or(best_position));

        // Nodes that are actually able to converge onto the most-work chain.
        let blocked_nodes: Vec<&str> = blocked_by_pruning
            .iter()
            .filter_map(|entry| entry["node"].as_str())
            .collect();
        let eligible: Vec<&NodeState> = states
            .iter()
            .filter(|state| !blocked_nodes.contains(&state.name.as_str()))
            .collect();

        // The most-work node is never blocked by pruning, so there is always at
        // least one eligible node.
        let first = eligible
            .first()
            .expect("the most-work node is never blocked by pruning");
        hashes_converged = eligible
            .iter()
            .all(|state| state.info.bestblockhash == first.info.bestblockhash);
        heights_converged = eligible
            .iter()
            .all(|state| state.info.blocks == first.info.blocks);
    }

    Round {
        expected,
        states,
        blocked_by_pruning,
        hashes_converged,
        heights_converged,
        extend,
    }
}

/// Address the driver mines to.
///
/// A v0 witness program of zeroes: a valid, standard output that no wallet can
/// ever spend. Mining to a node's own wallet would need a wallet to be loaded,
/// which another driver may have just unloaded, and would move the balances that
/// the wallet properties are watching.
fn mining_address() -> String {
    let program = WitnessProgram::new(WitnessVersion::V0, &[0u8; 20])
        .expect("a 20 byte v0 witness program is valid");
    Address::from_witness_program(program, Network::Regtest).to_string()
}

/// Highest block height any node knows a chain for, including the forks it isn't
/// following and the ones it only has headers of.
///
/// Every regtest block carries the same amount of work, so a chain one block
/// above this is the most-work chain in the cluster. Invalid tips are left out:
/// nobody will ever follow them, and the invalid block drivers can put them
/// arbitrarily far ahead.
fn highest_known_height(clients: &[(String, Client)]) -> u64 {
    clients
        .iter()
        .filter_map(|(_, client)| get_chain_tips(client).ok())
        .flatten()
        .filter(|tip| tip.status != "invalid")
        .map(|tip| tip.height)
        .max()
        .unwrap_or(0)
}

/// Extend a node's chain by `blocks` blocks of work, returning the last block
/// that was mined, which is the tip the rest of the cluster now has to follow.
fn extend_chain(name: &str, client: &Client, address: &str, blocks: u64) -> Option<String> {
    match client.call::<Vec<String>>(
        "generatetoaddress",
        &[serde_json::json!(blocks), serde_json::json!(address)],
    ) {
        Ok(hashes) => {
            let tip = hashes.last().cloned();
            println!(
                "Extended {}'s chain by {} block(s), new tip {}",
                name,
                hashes.len(),
                tip.as_deref().unwrap_or("<none>")
            );
            tip
        }
        Err(e) => {
            eprintln!("{} generatetoaddress failed: {}", name, e);
            None
        }
    }
}

fn assert_convergence(round: &Round, rounds: u64, elapsed: Duration, mined_tip: Option<&str>) {
    let some_nodes_unavailable = !round.complete();
    let any_pruning_blocked = !round.blocked_by_pruning.is_empty();
    let fully_converged = round.fully_converged();
    let same_height_block_race = round.same_height_block_race();

    antithesis_sdk::assert_sometimes!(
        same_height_block_race,
        "Nodes are at the same height but have different chain tips",
        &serde_json::json!({
            "block_hashes": round.block_hashes(),
            "block_heights": round.block_heights(),
        })
    );

    antithesis_sdk::assert_sometimes!(
        round.complete() && fully_converged && !any_pruning_blocked,
        "All nodes are up and have converged to the same chain tip and height",
        &serde_json::json!({
            "block_hashes": round.block_hashes(),
            "block_heights": round.block_heights(),
        })
    );

    // Eventually all nodes should be on the same chain tip.
    //
    // Exceptions:
    //   - Some nodes are unavailable, which will be caught and reported by other property failures
    //   - All nodes are at the same height but have different tips, which can occur and is benign
    //     in the case of same height block races
    //   - Pruning has made following the most-work chain impossible for a node, either because it
    //     would have to disconnect a block it already pruned, or because every node on that chain
    //     has pruned the blocks above the fork point (such nodes are excluded from the convergence
    //     check)
    antithesis_sdk::assert_always!(
        some_nodes_unavailable || same_height_block_race || fully_converged,
        "Some nodes are unavailable, a same height block race occured, or all nodes have converged to the same chain tip",
        &serde_json::json!({
            "block_hashes": round.block_hashes(),
            "block_heights": round.block_heights(),
            "prune_heights": round.prune_heights(),
            "some_nodes_unavailable": some_nodes_unavailable,
            "same_height_block_race": same_height_block_race,
            "fully_converged": fully_converged,
            "blocked_by_pruning": round.blocked_by_pruning,
            "last_mined_block": mined_tip,
            "all_at_last_mined_block": round.all_at(mined_tip),
            "rounds": rounds,
            "seconds": elapsed.as_secs(),
        })
    );
}
