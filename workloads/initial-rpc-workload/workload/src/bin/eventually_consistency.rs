use bitcoin_antithesis_workload::{create_client, get_all_nodes};
use std::thread;
use std::time::Duration;

fn main() {
    let nodes = get_all_nodes();

    // Give the nodes some time to sync after faults stop, then take a single
    // snapshot and judge all properties on it.
    thread::sleep(Duration::from_secs(60));

    let mut block_hashes: Vec<(String, String)> = Vec::new();
    let mut block_heights: Vec<(String, u64)> = Vec::new();
    let mut all_reachable = true;

    for (i, node_config) in nodes.iter().enumerate() {
        let client = match create_client(node_config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("node{} client creation failed: {}", i + 1, e);
                all_reachable = false;
                continue;
            }
        };
        match client.call::<String>("getbestblockhash", &[]) {
            Ok(hash) => {
                block_hashes.push((format!("node{}", i + 1), hash));
            }
            Err(e) => {
                eprintln!("node{} not reachable: {}", i + 1, e);
                all_reachable = false;
            }
        }

        match client.call::<u64>("getblockcount", &[]) {
            Ok(height) => {
                block_heights.push((format!("node{}", i + 1), height));
            }
            Err(e) => {
                eprintln!("node{} getblockcount failed: {}", i + 1, e);
                all_reachable = false;
            }
        }
    }

    let snapshot_complete = all_reachable
        && block_hashes.len() == nodes.len()
        && block_heights.len() == nodes.len();
    let some_nodes_unavailable = !snapshot_complete;
    let hashes_converged = snapshot_complete && {
        let first = &block_hashes[0].1;
        block_hashes.iter().all(|(_, hash)| hash == first)
    };
    let heights_converged = snapshot_complete && {
        let first = block_heights[0].1;
        block_heights.iter().all(|(_, height)| *height == first)
    };
    let fully_converged = hashes_converged && heights_converged;
    let same_height_block_race = heights_converged && !hashes_converged;

    println!("Snapshot:");
    for (node, hash) in &block_hashes {
        println!("  {}: {}", node, hash);
    }

    antithesis_sdk::assert_sometimes!(
        same_height_block_race,
        "Nodes are at the same height but have different chain tips",
        &serde_json::json!({
            "block_hashes": block_hashes,
            "block_heights": block_heights,
        })
    );

    antithesis_sdk::assert_sometimes!(
        fully_converged,
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
    antithesis_sdk::assert_always!(
        some_nodes_unavailable || same_height_block_race || fully_converged,
        "Some nodes are unavailable, a same height block race occured, or all nodes have converged to the same chain tip",
        &serde_json::json!({
            "block_hashes": block_hashes,
            "block_heights": block_heights,
            "some_nodes_unavailable": some_nodes_unavailable,
            "same_height_block_race": same_height_block_race,
            "fully_converged": fully_converged
        })
    );
}
