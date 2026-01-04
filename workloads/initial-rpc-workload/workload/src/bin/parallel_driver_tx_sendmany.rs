use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, get_all_nodes, random_node,
    random_range, round_to_satoshis,
};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[sendmany] Failed to create client: {}", e);
            return;
        }
    };

    let num_outputs = 2 + random_range(4) as usize; // 2-5 outputs
    let mut destinations = serde_json::Map::new();

    for _ in 0..num_outputs {
        let address: String = match client.call("getnewaddress", &[]) {
            Ok(addr) => addr,
            Err(e) => {
                eprintln!("[sendmany] Failed to get address: {}", e);
                return;
            }
        };
        let amount = round_to_satoshis(0.001 + (random_range(100) as f64) * 0.001);
        destinations.insert(address, json!(amount));
    }

    match client.call::<String>("sendmany", &[json!(""), json!(destinations)]) {
        Ok(txid) => {
            println!("[sendmany] Sent to {} addresses: {}", num_outputs, txid);
            assert_mempool_metrics(&client, "after_sendmany");
            assert_wallet_metrics(&client, "after_sendmany");
        }
        Err(e) => eprintln!("[sendmany] Failed: {}", e),
    }
}
