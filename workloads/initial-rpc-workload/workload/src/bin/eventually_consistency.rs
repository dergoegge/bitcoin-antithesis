use bitcoin_antithesis_workload::{create_client, get_all_nodes};
use std::thread;
use std::time::Duration;

fn main() {
    let nodes = get_all_nodes();

    // Allow some time for nodes to sync after faults stop
    // Retry a few times with delays
    let mut hashes_converged = false;
    let mut heights_converged = false;
    let mut block_hashes: Vec<(String, String)> = Vec::new();
    let mut block_heights: Vec<(String, u64)> = Vec::new();

    for attempt in 1..=40 {
        block_hashes.clear();
        let mut all_reachable = true;

        for (i, node_config) in nodes.iter().enumerate() {
            let client = match create_client(node_config) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "Attempt {}: node{} client creation failed: {}",
                        attempt,
                        i + 1,
                        e
                    );
                    all_reachable = false;
                    continue;
                }
            };
            match client.call::<String>("getbestblockhash", &[]) {
                Ok(hash) => {
                    block_hashes.push((format!("node{}", i + 1), hash));
                }
                Err(e) => {
                    eprintln!("Attempt {}: node{} not reachable: {}", attempt, i + 1, e);
                    all_reachable = false;
                }
            }

            match client.call::<u64>("getblockcount", &[]) {
                Ok(height) => {
                    block_heights.push((format!("node{}", i + 1), height));
                }
                Err(e) => {
                    eprintln!(
                        "Attempt {}: node{} getblockcount failed: {}",
                        attempt,
                        i + 1,
                        e
                    );
                    all_reachable = false;
                }
            }
        }

        if !all_reachable {
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let first_hash = &block_hashes[0].1;
        let all_hashes_same = block_hashes.iter().all(|(_, hash)| hash == first_hash);
        let first_height = block_heights[0].1;
        let all_heights_same = block_heights
            .iter()
            .all(|(_, height)| *height == first_height);

        if all_hashes_same && !hashes_converged {
            println!("All nodes converged to the same chain tip: {}", first_hash);
        }
        if all_heights_same && !heights_converged {
            println!("All nodes have the same block height: {}", first_height);
        }

        hashes_converged |= all_hashes_same;
        heights_converged |= all_heights_same;

        if hashes_converged && heights_converged {
            break;
        }

        // Log current state
        println!("Attempt {}: nodes not yet consistent:", attempt);
        for (node, hash) in &block_hashes {
            println!("  {}: {}", node, hash);
        }

        thread::sleep(Duration::from_secs(30));
    }

    let some_nodes_unavailable = block_hashes.len() < nodes.len();
    let same_height_block_race =
        heights_converged && !hashes_converged && (block_hashes.len() == nodes.len());
    antithesis_sdk::assert_sometimes!(
        same_height_block_race,
        "Nodes are at the same height but have different chain tips",
        &serde_json::json!({
            "block_hashes": block_hashes,
            "block_heights": block_heights,
        })
    );

    let fully_converged = hashes_converged && heights_converged;
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

    // TODO: test that races resolve
}
