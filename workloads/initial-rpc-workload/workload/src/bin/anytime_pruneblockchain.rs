use bitcoin_antithesis_workload::{
    create_client, get_all_nodes, get_blockchain_info, random_range, NodeConfig,
};

/// The wrapper decides per boot which nodes prune, so find one that does.
fn pruned_node(nodes: &[NodeConfig]) -> Option<&NodeConfig> {
    let pruned: Vec<&NodeConfig> = nodes
        .iter()
        .filter(|n| {
            create_client(n)
                .ok()
                .and_then(|c| get_blockchain_info(&c).ok())
                .is_some_and(|info| info.pruneheight.is_some())
        })
        .collect();

    if pruned.is_empty() {
        return None;
    }
    Some(pruned[random_range(pruned.len() as u64) as usize])
}

fn main() {
    let nodes = get_all_nodes();
    let Some(node) = pruned_node(&nodes) else {
        println!("[pruneblockchain] No node is in prune mode");
        return;
    };

    let client = match create_client(node) {
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
