use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, get_all_nodes, random_amount,
    random_node,
};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[simple_send] Failed to create client: {}", e);
            return;
        }
    };

    let address: String = match client.call("getnewaddress", &[]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[simple_send] Failed to get new address: {}", e);
            return;
        }
    };

    let amount = random_amount();

    match client.call::<String>("sendtoaddress", &[json!(address), json!(amount)]) {
        Ok(txid) => {
            println!("[simple_send] Sent {} BTC to {}: {}", amount, address, txid);

            // Check mempool and wallet state after sending transaction
            assert_mempool_metrics(&client, "after_simple_send");
            assert_wallet_metrics(&client, "after_simple_send");
        }
        Err(e) => eprintln!("[simple_send] Failed to send: {}", e),
    }
}
