use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, get_all_nodes, random_node,
    random_range,
};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[prioritise_tx] Failed to create client: {}", e);
            return;
        }
    };

    // Get mempool transactions
    let mempool_txids: Vec<String> = match client.call("getrawmempool", &[]) {
        Ok(txids) => txids,
        Err(e) => {
            eprintln!("[prioritise_tx] Failed to get mempool: {}", e);
            return;
        }
    };

    if mempool_txids.is_empty() {
        eprintln!("[prioritise_tx] Mempool is empty");
        return;
    }

    // Pick a random transaction from the mempool
    let txid = &mempool_txids[random_range(mempool_txids.len() as u64) as usize];

    // Generate a random fee_delta between -100000 and 100000 satoshis
    let fee_delta = (random_range(200001) as i64) - 100000;

    match client.call::<bool>(
        "prioritisetransaction",
        &[json!(txid), json!(0), json!(fee_delta)],
    ) {
        Ok(result) => {
            println!(
                "[prioritise_tx] Prioritised tx {} with fee_delta {}: {}",
                txid, fee_delta, result
            );
            assert_mempool_metrics(&client, "after_prioritise_tx");
            assert_wallet_metrics(&client, "after_prioritise_tx");
        }
        Err(e) => {
            eprintln!(
                "[prioritise_tx] Failed to prioritise tx {} with fee_delta {}: {}",
                txid, fee_delta, e
            );
        }
    }
}
