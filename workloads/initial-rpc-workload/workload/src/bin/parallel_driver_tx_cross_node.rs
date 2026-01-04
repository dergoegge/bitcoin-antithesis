use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, get_all_nodes, random_amount,
    random_range,
};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();

    if nodes.len() < 2 {
        eprintln!("[cross_node] Need at least 2 nodes");
        return;
    }

    let sender_idx = random_range(nodes.len() as u64) as usize;
    let mut receiver_idx = random_range(nodes.len() as u64) as usize;
    if receiver_idx == sender_idx {
        receiver_idx = (receiver_idx + 1) % nodes.len();
    }

    let sender_client = match create_client(&nodes[sender_idx]) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[cross_node] Failed to create sender client: {}", e);
            return;
        }
    };
    let receiver_client = match create_client(&nodes[receiver_idx]) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[cross_node] Failed to create receiver client: {}", e);
            return;
        }
    };

    let address: String = match receiver_client.call("getnewaddress", &[]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[cross_node] Failed to get receiver address: {}", e);
            return;
        }
    };

    let amount = random_amount();

    match sender_client.call::<String>("sendtoaddress", &[json!(address), json!(amount)]) {
        Ok(txid) => {
            println!(
                "[cross_node] Sent {} BTC from node {} to node {}: {}",
                amount, sender_idx, receiver_idx, txid
            );
            assert_mempool_metrics(&sender_client, "after_cross_node");
            assert_wallet_metrics(&sender_client, "after_cross_node");
        }
        Err(e) => eprintln!("[cross_node] Failed to send: {}", e),
    }
}
