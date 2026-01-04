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
            eprintln!("[raw_tx] Failed to create client: {}", e);
            return;
        }
    };

    // Get blockchain info for locktime calculation
    // We need block height and median time past (MTP) - Bitcoin validates timestamp
    // locktimes against MTP, not current time
    let blockchain_info: serde_json::Value = match client.call("getblockchaininfo", &[]) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("[raw_tx] Failed to get blockchain info: {}", e);
            return;
        }
    };

    let block_count = blockchain_info["blocks"].as_u64().unwrap_or(0);
    let median_time = blockchain_info["mediantime"].as_u64().unwrap_or(500000000) as u32;

    // Get UTXOs
    let utxos: Vec<serde_json::Value> =
        match client.call("listunspent", &[json!(1), json!(9999999)]) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[raw_tx] Failed to list unspent: {}", e);
                return;
            }
        };

    if utxos.is_empty() {
        eprintln!("[raw_tx] No UTXOs available");
        return;
    }

    // Pick a random UTXO
    let utxo = &utxos[random_range(utxos.len() as u64) as usize];
    let txid = utxo["txid"].as_str().unwrap();
    let vout = utxo["vout"].as_u64().unwrap();
    let utxo_amount = utxo["amount"].as_f64().unwrap();

    // Create output address
    let address: String = match client.call("getnewaddress", &[]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[raw_tx] Failed to get address: {}", e);
            return;
        }
    };

    // Calculate amount (leave some for fee)
    let send_amount = round_to_satoshis((utxo_amount - 0.0001).max(0.0001));

    // Fuzz version (RPC only allows 1-3)
    let version = match random_range(3) {
        0 => 1, // Version 1
        1 => 2, // Version 2 (BIP68)
        _ => 3, // Version 3 (TRUC)
    };

    // Fuzz locktime using current block height and median time past (MTP)
    // Block height locktime: < 500000000, must be <= current block height
    // Timestamp locktime: >= 500000000, must be <= MTP (not current time!)
    let locktime: u32 = match random_range(5) {
        0 => 0,                                                                     // No locktime
        1 => random_range(block_count.min(500000000 - 1) + 1) as u32, // Random past block height
        2 => block_count.min(500000000 - 1) as u32,                   // Current block height
        3 => 500000000 + random_range((median_time - 500000000) as u64 + 1) as u32, // Random past timestamp
        _ => median_time,                                                           // Current MTP
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

    // Create transaction with fuzzed values
    let inputs = json!([{"txid": txid, "vout": vout, "sequence": sequence}]);
    let outputs = json!([{address: send_amount}]);

    let raw_tx: String = match client.call(
        "createrawtransaction",
        &[
            inputs,
            outputs,
            json!(locktime),
            json!(false),
            json!(version),
        ],
    ) {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!(
                "[raw_tx] Failed to create (version={}, locktime={}, seq={:#x}): {}",
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
            eprintln!("[raw_tx] Failed to sign: {}", e);
            return;
        }
    };

    if !signed.complete {
        eprintln!("[raw_tx] Transaction signing incomplete");
        return;
    }

    // Send the transaction
    match client.call::<String>("sendrawtransaction", &[json!(signed.hex)]) {
        Ok(new_txid) => {
            println!(
                "[raw_tx] Sent (version={}, locktime={}, seq={:#x}): {}",
                version, locktime, sequence, new_txid
            );
            assert_mempool_metrics(&client, "after_raw_tx");
            assert_wallet_metrics(&client, "after_raw_tx");
        }
        Err(e) => eprintln!(
            "[raw_tx] Failed to send (version={}, locktime={}, seq={:#x}): {}",
            version, locktime, sequence, e
        ),
    }
}
