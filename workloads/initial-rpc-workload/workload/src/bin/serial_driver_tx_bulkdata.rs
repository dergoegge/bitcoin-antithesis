use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, get_all_nodes, random_node,
    random_range, round_to_satoshis,
};
use serde_json::json;

const TARGET_BYTES: u64 = 6_000_000; // 6.0 MB
const MIN_OUTPUT_AMOUNT: f64 = 0.00001; // 1000 sats minimum per output
const MIN_CONFIRMED_UTXOS: usize = 50; // Minimum confirmed UTXOs before bulk phase

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[bulkdata] Failed to create client: {}", e);
            return;
        }
    };

    println!(
        "[bulkdata] Starting bulk transaction generation, target: {} bytes",
        TARGET_BYTES
    );

    // Phase 1: Setup - ensure we have enough confirmed UTXOs to work with
    setup_confirmed_utxos(&client);

    // Phase 2: Bulk - generate 6.0MB of transactions
    let (tx_count, total_bytes) = generate_bulk_transactions(&client);

    println!(
        "[bulkdata] Completed: {} transactions, {} bytes total",
        tx_count, total_bytes
    );

    assert_mempool_metrics(&client, "after_bulkdata");
    assert_wallet_metrics(&client, "after_bulkdata");
}

/// Phase 1: Create fan-out transactions and mine ONE block to get many confirmed UTXOs
fn setup_confirmed_utxos(client: &bitcoin_antithesis_workload::Client) {
    // Check current confirmed UTXO count
    let confirmed: Vec<serde_json::Value> = client
        .call("listunspent", &[json!(1), json!(9999999)])
        .unwrap_or_default();

    if confirmed.len() >= MIN_CONFIRMED_UTXOS {
        println!(
            "[bulkdata] Setup: Already have {} confirmed UTXOs, skipping setup",
            confirmed.len()
        );
        return;
    }

    println!(
        "[bulkdata] Setup: Have {} confirmed UTXOs, need {}",
        confirmed.len(),
        MIN_CONFIRMED_UTXOS
    );

    // Get all UTXOs including unconfirmed for setup
    let utxos: Vec<serde_json::Value> = client
        .call("listunspent", &[json!(0), json!(9999999)])
        .unwrap_or_default();

    if utxos.is_empty() {
        eprintln!("[bulkdata] Setup: No UTXOs available at all");
        return;
    }

    // Create fan-out transactions to generate more UTXOs, collecting their txids
    let mut fanout_txids: Vec<String> = Vec::new();
    for utxo in utxos.iter().take(10) {
        // Use up to 10 UTXOs for setup
        let amount = utxo["amount"].as_f64().unwrap_or(0.0);
        if amount < 0.01 {
            continue; // Skip small UTXOs
        }

        match create_setup_fanout(client, utxo) {
            Ok(txid) => {
                println!("[bulkdata] Setup: Created fanout tx {}", &txid[..8]);
                fanout_txids.push(txid);
            }
            Err(e) => {
                eprintln!("[bulkdata] Setup: Fanout failed: {}", e);
            }
        }
    }

    if fanout_txids.is_empty() {
        eprintln!("[bulkdata] Setup: No fanout transactions created");
        return;
    }

    println!(
        "[bulkdata] Setup: Created {} fanout txs, mining block with ONLY these txs",
        fanout_txids.len()
    );

    // Mine ONE block containing ONLY our fanout transactions (no other mempool txs)
    let address: Result<String, _> = client.call("getnewaddress", &[]);
    if let Ok(addr) = address {
        // Use generateblock to mine only specific transactions
        match client.call::<serde_json::Value>("generateblock", &[json!(addr), json!(fanout_txids)])
        {
            Ok(result) => {
                let hash = result["hash"].as_str().unwrap_or("?");
                println!(
                    "[bulkdata] Setup: Mined block ({}) with {} fanout txs only",
                    &hash[..8.min(hash.len())],
                    fanout_txids.len()
                );
            }
            Err(e) => {
                eprintln!(
                    "[bulkdata] Setup: Failed to mine block with generateblock: {}",
                    e
                );
            }
        }
    }

    // Verify we now have more confirmed UTXOs
    let confirmed_after: Vec<serde_json::Value> = client
        .call("listunspent", &[json!(1), json!(9999999)])
        .unwrap_or_default();
    println!(
        "[bulkdata] Setup: Now have {} confirmed UTXOs",
        confirmed_after.len()
    );
}

/// Create a setup fan-out transaction (1 input -> 20 outputs)
fn create_setup_fanout(
    client: &bitcoin_antithesis_workload::Client,
    utxo: &serde_json::Value,
) -> Result<String, String> {
    let txid = utxo["txid"].as_str().ok_or("missing txid")?;
    let vout = utxo["vout"].as_u64().ok_or("missing vout")?;
    let utxo_amount = utxo["amount"].as_f64().ok_or("missing amount")?;

    let num_outputs = 20; // Fixed 20 outputs for setup
    let fee = 0.0002;
    let amount_per_output = round_to_satoshis((utxo_amount - fee) / (num_outputs as f64));

    if amount_per_output < MIN_OUTPUT_AMOUNT {
        return Err("UTXO too small".to_string());
    }

    let mut outputs_map = serde_json::Map::new();
    for _ in 0..num_outputs {
        let addr: String = client
            .call("getnewaddress", &[])
            .map_err(|e| format!("getnewaddress: {}", e))?;
        outputs_map.insert(addr, json!(amount_per_output));
    }

    let inputs = json!([{"txid": txid, "vout": vout}]);
    let outputs = serde_json::Value::Object(outputs_map);

    let raw_tx: String = client
        .call("createrawtransaction", &[inputs, outputs])
        .map_err(|e| format!("createrawtransaction: {}", e))?;

    let signed = sign_and_send(client, &raw_tx)?;
    Ok(signed.0)
}

/// Phase 2: Generate bulk transactions until we hit 6.0MB
fn generate_bulk_transactions(client: &bitcoin_antithesis_workload::Client) -> (u64, u64) {
    let mut total_bytes: u64 = 0;
    let mut tx_count: u64 = 0;
    let mut consecutive_failures: u64 = 0;

    while total_bytes < TARGET_BYTES {
        // Get confirmed UTXOs - each one is an independent cluster root
        let utxos: Vec<serde_json::Value> =
            match client.call("listunspent", &[json!(1), json!(9999999)]) {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("[bulkdata] listunspent failed: {}", e);
                    consecutive_failures += 1;
                    if consecutive_failures > 50 {
                        eprintln!("[bulkdata] Too many failures, stopping");
                        break;
                    }
                    continue;
                }
            };

        if utxos.is_empty() {
            eprintln!("[bulkdata] No confirmed UTXOs available");
            consecutive_failures += 1;
            if consecutive_failures > 50 {
                break;
            }
            continue;
        }

        // Pick a random confirmed UTXO to use as cluster root
        let utxo_idx = random_range(utxos.len() as u64) as usize;
        let utxo = &utxos[utxo_idx];

        // Pick transaction type
        let tx_type = random_range(10);

        let result = match tx_type {
            0..=3 => create_fanout_tx(client, utxo), // 40% - fan out 1->many
            4..=5 => create_fanout_with_opreturn(client, utxo), // 20% - fan out with large OP_RETURN
            6..=7 => create_wide_opreturn_tx(client, utxo),     // 20% - large OP_RETURN + outputs
            8 => create_mixed_fanout_tx(client, utxo),          // 10% - mixed address types
            _ => create_mega_opreturn_tx(client, utxo),         // 10% - very large OP_RETURN
        };

        match result {
            Ok(bytes) => {
                total_bytes += bytes;
                tx_count += 1;
                consecutive_failures = 0;
                if tx_count % 20 == 0 {
                    println!(
                        "[bulkdata] Progress: {} txs, {} bytes ({:.1}%), {} confirmed UTXOs",
                        tx_count,
                        total_bytes,
                        (total_bytes as f64 / TARGET_BYTES as f64) * 100.0,
                        utxos.len()
                    );
                }
            }
            Err(e) => {
                if !e.contains("too-large-cluster") {
                    eprintln!("[bulkdata] TX failed (type {}): {}", tx_type, e);
                }
                consecutive_failures += 1;
                if consecutive_failures > 50 {
                    eprintln!("[bulkdata] Too many consecutive failures, stopping");
                    break;
                }
            }
        }
    }

    (tx_count, total_bytes)
}

/// Fan out: 1 input -> 5-20 outputs
fn create_fanout_tx(
    client: &bitcoin_antithesis_workload::Client,
    utxo: &serde_json::Value,
) -> Result<u64, String> {
    let txid = utxo["txid"].as_str().ok_or("missing txid")?;
    let vout = utxo["vout"].as_u64().ok_or("missing vout")?;
    let utxo_amount = utxo["amount"].as_f64().ok_or("missing amount")?;

    let num_outputs = 5 + random_range(16) as usize;
    let fee = 0.0001 + (num_outputs as f64) * 0.000005;
    let amount_per_output = round_to_satoshis((utxo_amount - fee) / (num_outputs as f64));

    if amount_per_output < MIN_OUTPUT_AMOUNT {
        return Err("UTXO too small for fanout".to_string());
    }

    let mut outputs_map = serde_json::Map::new();
    for _ in 0..num_outputs {
        let addr: String = client
            .call("getnewaddress", &[])
            .map_err(|e| format!("getnewaddress: {}", e))?;
        outputs_map.insert(addr, json!(amount_per_output));
    }

    let inputs = json!([{"txid": txid, "vout": vout}]);
    let outputs = serde_json::Value::Object(outputs_map);

    let raw_tx: String = client
        .call("createrawtransaction", &[inputs, outputs])
        .map_err(|e| format!("createrawtransaction: {}", e))?;

    let signed = sign_and_send(client, &raw_tx)?;
    println!(
        "[bulkdata] Fanout tx (1->{} outputs): {}",
        num_outputs, signed.0
    );
    Ok(signed.1)
}

/// Fan out with large OP_RETURN: 1 input -> OP_RETURN (10-50KB) + 5-15 outputs
fn create_fanout_with_opreturn(
    client: &bitcoin_antithesis_workload::Client,
    utxo: &serde_json::Value,
) -> Result<u64, String> {
    let txid = utxo["txid"].as_str().ok_or("missing txid")?;
    let vout = utxo["vout"].as_u64().ok_or("missing vout")?;
    let utxo_amount = utxo["amount"].as_f64().ok_or("missing amount")?;

    let num_outputs = 5 + random_range(11) as usize;
    let fee = 0.001 + (num_outputs as f64) * 0.000005;
    let amount_per_output = round_to_satoshis((utxo_amount - fee) / (num_outputs as f64));

    if amount_per_output < MIN_OUTPUT_AMOUNT {
        return Err("UTXO too small".to_string());
    }

    // Large OP_RETURN data (10KB - 50KB)
    let data_len = 10_000 + random_range(40_001) as usize;
    let data_hex = generate_random_hex(data_len);

    let mut outputs_vec: Vec<serde_json::Value> = Vec::new();
    outputs_vec.push(json!({"data": data_hex}));

    for _ in 0..num_outputs {
        let addr: String = client
            .call("getnewaddress", &[])
            .map_err(|e| format!("getnewaddress: {}", e))?;
        let mut m = serde_json::Map::new();
        m.insert(addr, json!(amount_per_output));
        outputs_vec.push(serde_json::Value::Object(m));
    }

    let inputs = json!([{"txid": txid, "vout": vout}]);
    let outputs = serde_json::Value::Array(outputs_vec);

    let raw_tx: String = client
        .call("createrawtransaction", &[inputs, outputs])
        .map_err(|e| format!("createrawtransaction: {}", e))?;

    let signed = sign_and_send(client, &raw_tx)?;
    println!(
        "[bulkdata] Fanout+OP_RETURN tx ({} KB data, {} outputs): {}",
        data_len / 1000,
        num_outputs,
        signed.0
    );
    Ok(signed.1)
}

/// Wide OP_RETURN: large OP_RETURN (50-100KB) + many outputs
fn create_wide_opreturn_tx(
    client: &bitcoin_antithesis_workload::Client,
    utxo: &serde_json::Value,
) -> Result<u64, String> {
    let txid = utxo["txid"].as_str().ok_or("missing txid")?;
    let vout = utxo["vout"].as_u64().ok_or("missing vout")?;
    let utxo_amount = utxo["amount"].as_f64().ok_or("missing amount")?;

    let num_regular_outputs = 10 + random_range(21) as usize;
    let fee = 0.002 + (num_regular_outputs as f64) * 0.000005;
    let amount_per_output = round_to_satoshis((utxo_amount - fee) / (num_regular_outputs as f64));

    if amount_per_output < MIN_OUTPUT_AMOUNT {
        return Err("UTXO too small".to_string());
    }

    // Large OP_RETURN (50KB - 100KB)
    let data_len = 50_000 + random_range(50_001) as usize;
    let data_hex = generate_random_hex(data_len);

    let mut outputs_vec: Vec<serde_json::Value> = Vec::new();
    outputs_vec.push(json!({"data": data_hex}));

    for _ in 0..num_regular_outputs {
        let addr: String = client
            .call("getnewaddress", &[])
            .map_err(|e| format!("getnewaddress: {}", e))?;
        let mut m = serde_json::Map::new();
        m.insert(addr, json!(amount_per_output));
        outputs_vec.push(serde_json::Value::Object(m));
    }

    let inputs = json!([{"txid": txid, "vout": vout}]);
    let outputs = serde_json::Value::Array(outputs_vec);

    let raw_tx: String = client
        .call("createrawtransaction", &[inputs, outputs])
        .map_err(|e| format!("createrawtransaction: {}", e))?;

    let signed = sign_and_send(client, &raw_tx)?;
    println!(
        "[bulkdata] Wide OP_RETURN tx ({} KB data + {} outputs): {}",
        data_len / 1000,
        num_regular_outputs,
        signed.0
    );
    Ok(signed.1)
}

/// Mega OP_RETURN: very large OP_RETURN (80-100KB) + few outputs
fn create_mega_opreturn_tx(
    client: &bitcoin_antithesis_workload::Client,
    utxo: &serde_json::Value,
) -> Result<u64, String> {
    let txid = utxo["txid"].as_str().ok_or("missing txid")?;
    let vout = utxo["vout"].as_u64().ok_or("missing vout")?;
    let utxo_amount = utxo["amount"].as_f64().ok_or("missing amount")?;

    let num_outputs = 3 + random_range(5) as usize; // 3-7 outputs
    let fee = 0.003;
    let amount_per_output = round_to_satoshis((utxo_amount - fee) / (num_outputs as f64));

    if amount_per_output < MIN_OUTPUT_AMOUNT {
        return Err("UTXO too small".to_string());
    }

    // Very large OP_RETURN (80KB - 100KB)
    let data_len = 80_000 + random_range(20_001) as usize;
    let data_hex = generate_random_hex(data_len);

    let mut outputs_vec: Vec<serde_json::Value> = Vec::new();
    outputs_vec.push(json!({"data": data_hex}));

    for _ in 0..num_outputs {
        let addr: String = client
            .call("getnewaddress", &[])
            .map_err(|e| format!("getnewaddress: {}", e))?;
        let mut m = serde_json::Map::new();
        m.insert(addr, json!(amount_per_output));
        outputs_vec.push(serde_json::Value::Object(m));
    }

    let inputs = json!([{"txid": txid, "vout": vout}]);
    let outputs = serde_json::Value::Array(outputs_vec);

    let raw_tx: String = client
        .call("createrawtransaction", &[inputs, outputs])
        .map_err(|e| format!("createrawtransaction: {}", e))?;

    let signed = sign_and_send(client, &raw_tx)?;
    println!(
        "[bulkdata] Mega OP_RETURN tx ({} KB data + {} outputs): {}",
        data_len / 1000,
        num_outputs,
        signed.0
    );
    Ok(signed.1)
}

/// Mixed fanout with different address types
fn create_mixed_fanout_tx(
    client: &bitcoin_antithesis_workload::Client,
    utxo: &serde_json::Value,
) -> Result<u64, String> {
    let txid = utxo["txid"].as_str().ok_or("missing txid")?;
    let vout = utxo["vout"].as_u64().ok_or("missing vout")?;
    let utxo_amount = utxo["amount"].as_f64().ok_or("missing amount")?;

    let num_outputs = 8 + random_range(13) as usize;
    let fee = 0.0001 + (num_outputs as f64) * 0.000005;
    let amount_per_output = round_to_satoshis((utxo_amount - fee) / (num_outputs as f64));

    if amount_per_output < MIN_OUTPUT_AMOUNT {
        return Err("UTXO too small".to_string());
    }

    let addr_types = ["legacy", "p2sh-segwit", "bech32", "bech32m"];
    let mut outputs_map = serde_json::Map::new();

    for i in 0..num_outputs {
        let addr_type = addr_types[i % addr_types.len()];
        let addr: String = client
            .call("getnewaddress", &[json!(""), json!(addr_type)])
            .map_err(|e| format!("getnewaddress: {}", e))?;
        outputs_map.insert(addr, json!(amount_per_output));
    }

    let inputs = json!([{"txid": txid, "vout": vout}]);
    let outputs = serde_json::Value::Object(outputs_map);

    let raw_tx: String = client
        .call("createrawtransaction", &[inputs, outputs])
        .map_err(|e| format!("createrawtransaction: {}", e))?;

    let signed = sign_and_send(client, &raw_tx)?;
    println!(
        "[bulkdata] Mixed fanout tx ({} outputs, mixed types): {}",
        num_outputs, signed.0
    );
    Ok(signed.1)
}

/// Sign and send a raw transaction, returning (txid, size_in_bytes)
fn sign_and_send(
    client: &bitcoin_antithesis_workload::Client,
    raw_tx: &str,
) -> Result<(String, u64), String> {
    #[derive(serde::Deserialize)]
    struct SignResult {
        hex: String,
        complete: bool,
    }

    let signed: SignResult = client
        .call("signrawtransactionwithwallet", &[json!(raw_tx)])
        .map_err(|e| format!("signrawtransaction: {}", e))?;

    if !signed.complete {
        return Err("Signing incomplete".to_string());
    }

    let tx_bytes = (signed.hex.len() / 2) as u64;

    let txid: String = client
        .call("sendrawtransaction", &[json!(signed.hex)])
        .map_err(|e| format!("sendrawtransaction: {}", e))?;

    Ok((txid, tx_bytes))
}

/// Generate random hex string of specified byte length
fn generate_random_hex(num_bytes: usize) -> String {
    let mut hex = String::with_capacity(num_bytes * 2);
    for _ in 0..num_bytes {
        let byte = random_range(256) as u8;
        hex.push_str(&format!("{:02x}", byte));
    }
    hex
}
