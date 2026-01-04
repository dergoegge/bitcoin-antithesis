use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, get_all_nodes, random_amount,
    random_node, random_range,
};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[addr_types] Failed to create client: {}", e);
            return;
        }
    };

    let address_types = ["legacy", "p2sh-segwit", "bech32", "bech32m"];
    let addr_type = address_types[random_range(address_types.len() as u64) as usize];

    let address: String = match client.call("getnewaddress", &[json!(""), json!(addr_type)]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[addr_types] Failed to get {} address: {}", addr_type, e);
            return;
        }
    };

    let amount = random_amount();

    match client.call::<String>("sendtoaddress", &[json!(address), json!(amount)]) {
        Ok(txid) => {
            println!(
                "[addr_types] Sent {} BTC to {} ({}): {}",
                amount, address, addr_type, txid
            );
            assert_mempool_metrics(&client, "after_addr_types");
            assert_wallet_metrics(&client, "after_addr_types");
        }
        Err(e) => eprintln!("[addr_types] Failed to send: {}", e),
    }
}
