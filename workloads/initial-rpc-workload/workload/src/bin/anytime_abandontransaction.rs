use bitcoin_antithesis_workload::{create_client, get_all_nodes, random_node, random_range};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[abandontx] Failed to create client: {}", e);
            return;
        }
    };

    // Get recent transactions (up to 100)
    let transactions: Vec<serde_json::Value> =
        match client.call("listtransactions", &[json!("*"), json!(1000)]) {
            Ok(txs) => txs,
            Err(e) => {
                eprintln!("[abandontx] Failed to list transactions: {}", e);
                return;
            }
        };

    if transactions.is_empty() {
        eprintln!("[abandontx] No transactions found");
        return;
    }

    // Filter for unconfirmed transactions (confirmations == 0) that might be abandonable
    // abandontransaction only works on txs not in a block and not in mempool
    let unconfirmed: Vec<&serde_json::Value> = transactions
        .iter()
        .filter(|tx| {
            let confirmations = tx["confirmations"].as_i64().unwrap_or(1);
            confirmations == 0
        })
        .collect();

    if unconfirmed.is_empty() {
        // Try to abandon a random transaction anyway - it will fail gracefully
        let tx = &transactions[random_range(transactions.len() as u64) as usize];
        let txid = tx["txid"].as_str().unwrap_or("");

        match client.call::<serde_json::Value>("abandontransaction", &[json!(txid)]) {
            Ok(_) => println!("[abandontx] Abandoned transaction: {}", txid),
            Err(e) => eprintln!("[abandontx] Failed to abandon {} (expected): {}", txid, e),
        }
        return;
    }

    // Pick a random unconfirmed transaction
    let tx = unconfirmed[random_range(unconfirmed.len() as u64) as usize];
    let txid = tx["txid"].as_str().unwrap_or("");

    match client.call::<serde_json::Value>("abandontransaction", &[json!(txid)]) {
        Ok(_) => println!("[abandontx] Abandoned transaction: {}", txid),
        Err(e) => eprintln!("[abandontx] Failed to abandon {}: {}", txid, e),
    }
}
