use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, get_all_nodes, random_node,
    random_range, round_to_satoshis,
};
use serde_json::json;
use std::collections::HashSet;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[consolidate] Failed to create client: {}", e);
            return;
        }
    };

    // Get UTXOs
    let utxos: Vec<serde_json::Value> =
        match client.call("listunspent", &[json!(1), json!(9999999)]) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[consolidate] Failed to list unspent: {}", e);
                return;
            }
        };

    if utxos.len() < 2 {
        eprintln!("[consolidate] Need at least 2 UTXOs to consolidate");
        return;
    }

    // Pick 2-5 random UTXOs
    let num_inputs = (2 + random_range(4) as usize).min(utxos.len());
    let mut selected_utxos: Vec<&serde_json::Value> = Vec::new();
    let mut total_amount = 0.0;
    let mut used_indices = HashSet::new();

    while selected_utxos.len() < num_inputs {
        let idx = random_range(utxos.len() as u64) as usize;
        if used_indices.insert(idx) {
            selected_utxos.push(&utxos[idx]);
            total_amount += utxos[idx]["amount"].as_f64().unwrap();
        }
    }

    // Fuzz version (RPC only allows 1-3)
    let version = match random_range(3) {
        0 => 1, // Version 1
        1 => 2, // Version 2 (BIP68)
        _ => 3, // Version 3 (TRUC)
    };

    // Fuzz locktime (full u32 range allowed by RPC)
    let locktime: u32 = match random_range(6) {
        0 => 0,                                           // No locktime
        1 => random_range(500) as u32,                    // Low block height (likely valid)
        2 => random_range(500000000) as u32,              // Block height range
        3 => 500000000 + random_range(2000000000) as u32, // Timestamp range
        4 => 0xffffffff,                                  // Max locktime
        _ => random_range(0xffffffff) as u32,             // Fully random
    };

    // Fuzz sequence (full u32 range, use replaceable=false to bypass validation)
    let sequence: u32 = match random_range(6) {
        0 => 0xffffffff,                      // Final
        1 => 0xfffffffe,                      // Locktime enabled
        2 => 0xffffffff - 2,                  // RBF enabled
        3 => 0,                               // Zero sequence
        4 => random_range(0xffff) as u32,     // Low sequence (relative locktime range)
        _ => random_range(0xffffffff) as u32, // Fully random
    };

    // Create inputs with fuzzed sequence
    let inputs: Vec<serde_json::Value> = selected_utxos
        .iter()
        .map(|u| json!({"txid": u["txid"], "vout": u["vout"], "sequence": sequence}))
        .collect();

    // Create output address
    let address: String = match client.call("getnewaddress", &[]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[consolidate] Failed to get address: {}", e);
            return;
        }
    };

    // Calculate amount (leave some for fee)
    let send_amount = round_to_satoshis((total_amount - 0.0002).max(0.0001));

    let outputs = json!([{address: send_amount}]);

    // Create transaction with fuzzed values
    let raw_tx: String = match client.call(
        "createrawtransaction",
        &[
            json!(inputs),
            outputs,
            json!(locktime),
            json!(false),
            json!(version),
        ],
    ) {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!(
                "[consolidate] Failed to create (version={}, locktime={}, seq={:#x}): {}",
                version, locktime, sequence, e
            );
            return;
        }
    };

    // Sign the transaction
    #[derive(serde::Deserialize)]
    struct SignResult {
        hex: String,
        complete: bool,
    }

    let signed: SignResult = match client.call("signrawtransactionwithwallet", &[json!(raw_tx)]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[consolidate] Failed to sign: {}", e);
            return;
        }
    };

    if !signed.complete {
        eprintln!("[consolidate] Transaction signing incomplete");
        return;
    }

    // Send the transaction
    match client.call::<String>("sendrawtransaction", &[json!(signed.hex)]) {
        Ok(txid) => {
            println!(
                "[consolidate] Consolidated {} UTXOs (version={}, locktime={}, seq={:#x}): {}",
                num_inputs, version, locktime, sequence, txid
            );
            assert_mempool_metrics(&client, "after_consolidate");
            assert_wallet_metrics(&client, "after_consolidate");
        }
        Err(e) => eprintln!(
            "[consolidate] Failed to send (version={}, locktime={}, seq={:#x}): {}",
            version, locktime, sequence, e
        ),
    }
}
