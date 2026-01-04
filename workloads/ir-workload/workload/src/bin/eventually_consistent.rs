use ir_workload::{create_client, get_all_nodes};
use std::thread;
use std::time::Duration;

fn main() {
    let nodes = get_all_nodes();

    let mut converged = false;
    let mut block_hashes: Vec<(String, String)> = Vec::new();

    for attempt in 1..=40 {
        block_hashes.clear();
        let mut all_reachable = true;

        for (i, node_config) in nodes.iter().enumerate() {
            let client = match create_client(node_config) {
                Ok(c) => c,
                Err(e) => {
                    println!(
                        "eventually_consistent: attempt {}: node{} client error: {}",
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
                    println!(
                        "eventually_consistent: attempt {}: node{} not reachable: {}",
                        attempt,
                        i + 1,
                        e
                    );
                    all_reachable = false;
                }
            }
        }

        if !all_reachable || block_hashes.is_empty() {
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        // Check if all nodes have the same best block hash
        let first_hash = &block_hashes[0].1;
        let all_same = block_hashes.iter().all(|(_, hash)| hash == first_hash);

        if all_same {
            println!(
                "eventually_consistent: all nodes converged to the same chain tip: {}",
                first_hash
            );
            converged = true;
            break;
        }

        // Log current state
        println!(
            "eventually_consistent: attempt {}: nodes not yet consistent:",
            attempt
        );
        for (node, hash) in &block_hashes {
            println!("  {}: {}", node, hash);
        }

        thread::sleep(Duration::from_secs(30));
    }

    antithesis_sdk::assert_always!(
        converged,
        "All nodes have consistent chain tip",
        &serde_json::json!({
            "block_hashes": block_hashes
        })
    );
}
