use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_reorg_metrics, assert_wallet_metrics, create_client,
    get_all_nodes, random_node, random_range,
};

fn main() {
    let nodes = get_all_nodes();

    // Pick a random node to mine on
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create client: {}", e);
            return;
        }
    };

    // Get a new address for mining rewards
    let address: String = match client.call("getnewaddress", &[]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("Failed to get new address: {}", e);
            return;
        }
    };

    // Mine 1-3 blocks
    let num_blocks = 1 + random_range(3);

    match client.call::<Vec<String>>(
        "generatetoaddress",
        &[serde_json::json!(num_blocks), serde_json::json!(address)],
    ) {
        Ok(block_hashes) => {
            println!("Mined {} blocks: {:?}", block_hashes.len(), block_hashes);

            // Check for reorg activity after mining
            assert_reorg_metrics(&client, "after_mining");
            assert_mempool_metrics(&client, "after_mining");
            assert_wallet_metrics(&client, "after_mining");
        }
        Err(e) => {
            eprintln!("Failed to mine blocks: {}", e);
        }
    }
}
