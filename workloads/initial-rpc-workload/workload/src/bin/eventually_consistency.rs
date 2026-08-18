use bitcoin_antithesis_workload::{
    create_client, disconnect_blocked_by_pruning, download_blocked_by_pruning, find_fork_height,
    get_all_nodes, get_blockchain_info, set_network_active, BlockchainInfo, Client,
};
use std::thread;
use std::time::Duration;

fn main() {
    let nodes = get_all_nodes();

    let mut clients: Vec<(String, Client)> = Vec::new();
    let mut all_reachable = true;

    for (i, node_config) in nodes.iter().enumerate() {
        let name = format!("node{}", i + 1);
        match create_client(node_config) {
            Ok(c) => clients.push((name, c)),
            Err(e) => {
                eprintln!("{} client creation failed: {}", name, e);
                all_reachable = false;
            }
        }
    }

    // Force every node to drop and re-establish its connections, then give them
    // time to sync before taking a single snapshot and judging all properties
    // on it.
    for active in [false, true] {
        for (name, client) in clients.iter() {
            if let Err(e) = set_network_active(client, active) {
                eprintln!("{} setnetworkactive {} failed: {}", name, active, e);
                all_reachable = false;
            }
        }
        thread::sleep(Duration::from_secs(5));
    }

    thread::sleep(Duration::from_secs(120));

    let mut snapshots: Vec<(String, Client, BlockchainInfo)> = Vec::new();

    for (name, client) in clients {
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

    // Pruning can make convergence onto the most-work chain impossible in two
    // ways, from either side of the fork:
    //
    //   - the lagging node pruned the undo data of a block it has to disconnect,
    //     so it can never reorg away from its own chain
    //   - every node on the most-work chain pruned the blocks above the fork
    //     point, so the lagging node can never download that branch
    //
    // Both are pruning limitations rather than consistency violations, so those
    // nodes are excluded from the convergence check below.
    let mut pruning_blocked: Vec<serde_json::Value> = Vec::new();
    if snapshot_complete {
        let best_idx = (0..snapshots.len())
            .max_by(|&a, &b| snapshots[a].2.chainwork.cmp(&snapshots[b].2.chainwork))
            .expect("snapshot is complete, so there is at least one node");
        let (_, best_client, best_info) = &snapshots[best_idx];
        let best_tip = best_info.bestblockhash.clone();

        // The only nodes that hold the most-work chain's block data.
        let best_chain: Vec<&BlockchainInfo> = snapshots
            .iter()
            .map(|(_, _, info)| info)
            .filter(|info| info.bestblockhash == best_tip)
            .collect();

        for (name, client, info) in snapshots.iter() {
            if info.bestblockhash == best_tip {
                continue;
            }
            // The fork point is at or below the lagging node's tip height, so the
            // walk can't be longer than its own chain. Either side of the fork
            // can be missing the other's headers, so try both.
            let fork_height = find_fork_height(client, &best_tip, info.blocks + 1).or_else(|| {
                find_fork_height(best_client, &info.bestblockhash, info.blocks + 1)
            });
            let disconnect_blocked =
                fork_height.is_some_and(|height| disconnect_blocked_by_pruning(info, height));
            let download_blocked =
                fork_height.is_some_and(|height| download_blocked_by_pruning(height, &best_chain));

            let details = serde_json::json!({
                "node": name,
                "fork_height": fork_height,
                "pruneheight": info.pruneheight,
                "height": info.blocks,
                "tip": info.bestblockhash,
                "best_tip": best_tip,
                "best_height": best_info.blocks,
                "best_chain_prune_heights": best_chain
                    .iter()
                    .map(|info| info.pruneheight)
                    .collect::<Vec<Option<u64>>>(),
                "disconnect_blocked": disconnect_blocked,
                "download_blocked": download_blocked,
            });

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

            if disconnect_blocked || download_blocked {
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
    //   - Pruning has made following the most-work chain impossible for a node, either because it
    //     would have to disconnect a block it already pruned, or because every node on that chain
    //     has pruned the blocks above the fork point (such nodes are excluded from the convergence
    //     check)
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
            "blocked_by_pruning": pruning_blocked
        })
    );
}
