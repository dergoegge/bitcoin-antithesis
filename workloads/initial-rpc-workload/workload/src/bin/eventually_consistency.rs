use bitcoin_antithesis_workload::{
    create_client, get_all_nodes, get_blockchain_info, reorg_blocked_by_pruning, BlockchainInfo,
    Client,
};
use std::thread;
use std::time::Duration;

fn main() {
    let nodes = get_all_nodes();

    // Give the nodes some time to sync after faults stop, then take a single
    // snapshot and judge all properties on it.
    thread::sleep(Duration::from_secs(60));

    let mut snapshots: Vec<(String, Client, BlockchainInfo)> = Vec::new();
    let mut all_reachable = true;

    for (i, node_config) in nodes.iter().enumerate() {
        let name = format!("node{}", i + 1);
        let client = match create_client(node_config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{} client creation failed: {}", name, e);
                all_reachable = false;
                continue;
            }
        };
        match get_blockchain_info(&client) {
            Ok(info) => snapshots.push((name, client, info)),
            Err(e) => {
                eprintln!("{} getblockchaininfo failed: {}", name, e);
                all_reachable = false;
            }
        }
    }

    let snapshot_complete = all_reachable && snapshots.len() == nodes.len();
    let some_nodes_unavailable = !snapshot_complete;

    let block_hashes: Vec<(String, String)> = snapshots
        .iter()
        .map(|(name, _, info)| (name.clone(), info.bestblockhash.clone()))
        .collect();
    let block_heights: Vec<(String, u64)> = snapshots
        .iter()
        .map(|(name, _, info)| (name.clone(), info.blocks))
        .collect();
    let prune_heights: Vec<(String, u64)> = snapshots
        .iter()
        .filter_map(|(name, _, info)| info.pruneheight.map(|h| (name.clone(), h)))
        .collect();

    println!("Snapshot:");
    for (name, _, info) in &snapshots {
        println!(
            "  {}: height={} tip={} chainwork={} pruneheight={:?}",
            name, info.blocks, info.bestblockhash, info.chainwork, info.pruneheight
        );
    }

    // A pruned node can only reorg as far back as its block data reaches, so a
    // node that would have to disconnect an already pruned block to follow the
    // most-work chain is permanently stuck. That is a pruning limitation rather
    // than a consistency violation, so those nodes are excluded from the
    // convergence check below.
    let mut pruning_blocked: Vec<serde_json::Value> = Vec::new();
    if snapshot_complete {
        let best_idx = (0..snapshots.len())
            .max_by(|&a, &b| snapshots[a].2.chainwork.cmp(&snapshots[b].2.chainwork))
            .expect("snapshot is complete, so there is at least one node");
        let best_tip = snapshots[best_idx].2.bestblockhash.clone();

        for (i, (name, client, info)) in snapshots.iter().enumerate() {
            if i == best_idx || info.bestblockhash == best_tip {
                continue;
            }
            let fork_height = reorg_blocked_by_pruning(client, info, &best_tip);
            let details = serde_json::json!({
                "node": name,
                "fork_height": fork_height,
                "pruneheight": info.pruneheight,
                "height": info.blocks,
                "tip": info.bestblockhash,
                "best_tip": best_tip,
            });

            antithesis_sdk::assert_sometimes!(
                fork_height.is_some(),
                "A pruned node can't reorg onto the most-work chain because the fork point is below its pruneheight",
                &details
            );

            if fork_height.is_some() {
                pruning_blocked.push(details);
            }
        }
    }
    let blocked_nodes: Vec<&str> = pruning_blocked
        .iter()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    let any_pruning_blocked = !blocked_nodes.is_empty();

    // Nodes that are actually able to converge onto the most-work chain.
    let eligible: Vec<&(String, Client, BlockchainInfo)> = snapshots
        .iter()
        .filter(|(name, _, _)| !blocked_nodes.contains(&name.as_str()))
        .collect();

    let hashes_converged = snapshot_complete && {
        let first = &eligible[0].2.bestblockhash;
        eligible
            .iter()
            .all(|(_, _, info)| &info.bestblockhash == first)
    };
    let heights_converged = snapshot_complete && {
        let first = eligible[0].2.blocks;
        eligible.iter().all(|(_, _, info)| info.blocks == first)
    };
    let fully_converged = hashes_converged && heights_converged;
    let same_height_block_race = heights_converged && !hashes_converged;

    antithesis_sdk::assert_sometimes!(
        same_height_block_race,
        "Nodes are at the same height but have different chain tips",
        &serde_json::json!({
            "block_hashes": block_hashes,
            "block_heights": block_heights,
        })
    );

    antithesis_sdk::assert_sometimes!(
        fully_converged && !any_pruning_blocked,
        "All nodes are up and have converged to the same chain tip and height",
        &serde_json::json!({
            "block_hashes": block_hashes,
            "block_heights": block_heights,
        })
    );

    // Eventually all nodes should be on the same chain tip.
    //
    // Exceptions:
    //   - Some nodes are unavailable, which will be caught and reported by other property failures
    //   - All nodes are at the same height but have different tips, which can occur and is benign
    //     in the case of same height block races
    //   - A pruned node would have to disconnect a block it already pruned to follow the most-work
    //     chain, which it can never do (such nodes are excluded from the convergence check)
    antithesis_sdk::assert_always!(
        some_nodes_unavailable || same_height_block_race || fully_converged,
        "Some nodes are unavailable, a same height block race occured, or all nodes have converged to the same chain tip",
        &serde_json::json!({
            "block_hashes": block_hashes,
            "block_heights": block_heights,
            "prune_heights": prune_heights,
            "some_nodes_unavailable": some_nodes_unavailable,
            "same_height_block_race": same_height_block_race,
            "fully_converged": fully_converged,
            "reorg_blocked_by_pruning": pruning_blocked
        })
    );
}
