use bitcoin_antithesis_workload::{create_client, random_range, NodeConfig};

fn main() {
    // Connect to node2 specifically (the pruned node)
    let node2 = NodeConfig::from_env("NODE2");
    let client = match create_client(&node2) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[pruneblockchain] Failed to create client: {}", e);
            return;
        }
    };

    // Get current block height
    let height: u64 = match client.call("getblockcount", &[]) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[pruneblockchain] Failed to get block count: {}", e);
            return;
        }
    };

    if height < 2 {
        println!(
            "[pruneblockchain] Chain too short to prune (height={})",
            height
        );
        return;
    }

    // Random height to prune up to (1 to height-1)
    let prune_height = 1 + random_range(height - 1);

    match client.call::<u64>("pruneblockchain", &[serde_json::json!(prune_height)]) {
        Ok(pruned_to) => {
            println!("[pruneblockchain] Pruned to block {}", pruned_to);
        }
        Err(e) => {
            eprintln!(
                "[pruneblockchain] Failed to prune to {}: {}",
                prune_height, e
            );
        }
    }
}
