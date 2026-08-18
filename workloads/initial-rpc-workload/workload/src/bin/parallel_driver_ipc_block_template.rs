use bitcoin::consensus::encode::serialize;
use bitcoin::hashes::Hash;
use bitcoin::transaction::{OutPoint, TxIn, Version};
use bitcoin::{Amount, ScriptBuf, Sequence, Transaction, TxOut, Txid, Witness};
use bitcoin_antithesis_workload::{get_all_ipc_nodes, ipc, random_ipc_node, random_range};
use bitcoin_capnp_types::mining_capnp::block_create_options;
use tokio::task::LocalSet;

/// Build a coinbase Transaction from CoinbaseTx IPC fields.
fn build_coinbase_tx(
    version: u32,
    script_sig_prefix: &[u8],
    sequence: u32,
    required_outputs: &[Vec<u8>],
    witness: &[u8],
    lock_time: u32,
    block_reward_remaining: i64,
) -> Transaction {
    let coinbase_input = TxIn {
        previous_output: OutPoint::new(Txid::all_zeros(), 0xffffffff),
        script_sig: ScriptBuf::from_bytes(script_sig_prefix.to_vec()),
        sequence: Sequence(sequence),
        // `witness` is the raw 32-byte BIP141 witness reserved value, i.e. a
        // single witness stack element — not a serialized witness. It must be
        // preserved: with a witness commitment among the required outputs, a
        // coinbase without it fails bad-witness-nonce-size.
        witness: if witness.is_empty() {
            Witness::new()
        } else {
            Witness::from_slice(&[witness])
        },
    };

    // Deserialize required_outputs (each is a serialized TxOut)
    let mut outputs: Vec<TxOut> = required_outputs
        .iter()
        .filter_map(|raw| bitcoin::consensus::encode::deserialize(raw).ok())
        .collect();

    // Add an OP_RETURN output claiming the remaining block reward
    if block_reward_remaining > 0 {
        outputs.push(TxOut {
            value: Amount::from_sat(block_reward_remaining as u64),
            script_pubkey: ScriptBuf::new_op_return(&[]),
        });
    }

    Transaction {
        version: Version(version as i32),
        lock_time: bitcoin::locktime::absolute::LockTime::from_consensus(lock_time),
        input: vec![coinbase_input],
        output: outputs,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let nodes = get_all_ipc_nodes();
    let node = random_ipc_node(&nodes);

    LocalSet::new()
        .run_until(async {
            let (init_client, thread) = match ipc::bootstrap(&node.socket_path).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("IPC bootstrap failed: {}", e);
                    return;
                }
            };

            let mining_client = match ipc::make_mining(&init_client, &thread).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to create mining client: {}", e);
                    return;
                }
            };

            // Create a new block template
            let mut req = mining_client.create_new_block_request();
            req.get().get_context().unwrap().set_thread(thread.clone());
            {
                let mut options: block_create_options::Builder = req.get().init_options();
                options.set_use_mempool(true);
            }
            let template_client = match req.send().promise.await {
                Ok(response) => match response.get() {
                    Ok(result) => match result.get_result() {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("Failed to get block template: {}", e);
                            return;
                        }
                    },
                    Err(e) => {
                        eprintln!("createNewBlock response error: {}", e);
                        return;
                    }
                },
                Err(e) => {
                    eprintln!("createNewBlock request failed: {}", e);
                    return;
                }
            };

            // getBlockHeader - should be 80 bytes; keep the decoded header
            // around to reuse its version/timestamp for submitSolution.
            let template_header: Option<bitcoin::block::Header> = {
                let mut req = template_client.get_block_header_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => match result.get_result() {
                            Ok(header) => {
                                let len = header.len();
                                antithesis_sdk::assert_always!(
                                    len == 80,
                                    "IPC block header is 80 bytes",
                                    &serde_json::json!({ "header_len": len })
                                );
                                println!("getBlockHeader: {} bytes", len);
                                bitcoin::consensus::encode::deserialize(header).ok()
                            }
                            Err(e) => {
                                eprintln!("getBlockHeader result error: {}", e);
                                None
                            }
                        },
                        Err(e) => {
                            eprintln!("getBlockHeader response error: {}", e);
                            None
                        }
                    },
                    Err(e) => {
                        eprintln!("getBlockHeader request failed: {}", e);
                        None
                    }
                }
            };

            // getBlock - should be >= 80 bytes
            {
                let mut req = template_client.get_block_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => match result.get_result() {
                            Ok(block) => {
                                let len = block.len();
                                antithesis_sdk::assert_always!(
                                    len >= 80,
                                    "IPC block is at least 80 bytes",
                                    &serde_json::json!({ "block_len": len })
                                );
                                println!("getBlock: {} bytes", len);
                            }
                            Err(e) => eprintln!("getBlock result error: {}", e),
                        },
                        Err(e) => eprintln!("getBlock response error: {}", e),
                    },
                    Err(e) => eprintln!("getBlock request failed: {}", e),
                }
            }

            // getTxFees
            {
                let mut req = template_client.get_tx_fees_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => match result.get_result() {
                            Ok(fees) => {
                                let count = fees.len();
                                antithesis_sdk::assert_sometimes_greater_than!(
                                    count,
                                    1,
                                    "IPC block template has more than 1 tx fee (mempool txs)",
                                    &serde_json::json!({ "tx_fee_count": count })
                                );
                                println!("getTxFees: {} entries", count);
                            }
                            Err(e) => eprintln!("getTxFees result error: {}", e),
                        },
                        Err(e) => eprintln!("getTxFees response error: {}", e),
                    },
                    Err(e) => eprintln!("getTxFees request failed: {}", e),
                }
            }

            // getCoinbaseTx - fetch fields to build coinbase for submitSolution
            let coinbase_data = {
                let mut req = template_client.get_coinbase_tx_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => match result.get_result() {
                            Ok(cb) => {
                                let reward = cb.get_block_reward_remaining();
                                antithesis_sdk::assert_always!(
                                    reward >= 0,
                                    "IPC coinbase block_reward_remaining is non-negative",
                                    &serde_json::json!({ "block_reward_remaining": reward })
                                );

                                let script_sig_prefix = cb.get_script_sig_prefix().unwrap_or(&[]);
                                let witness_data = cb.get_witness().unwrap_or(&[]);
                                let required_outputs_reader = cb.get_required_outputs().unwrap();
                                let required_outputs: Vec<Vec<u8>> = (0..required_outputs_reader
                                    .len())
                                    .filter_map(|i| {
                                        required_outputs_reader.get(i).ok().map(|d| d.to_vec())
                                    })
                                    .collect();

                                let tx = build_coinbase_tx(
                                    cb.get_version(),
                                    script_sig_prefix,
                                    cb.get_sequence(),
                                    &required_outputs,
                                    witness_data,
                                    cb.get_lock_time(),
                                    reward,
                                );
                                let serialized = serialize(&tx);
                                println!(
                                    "getCoinbaseTx: version={}, reward={}, serialized={} bytes",
                                    cb.get_version(),
                                    reward,
                                    serialized.len()
                                );
                                Some(serialized)
                            }
                            Err(e) => {
                                eprintln!("getCoinbaseTx result error: {}", e);
                                None
                            }
                        },
                        Err(e) => {
                            eprintln!("getCoinbaseTx response error: {}", e);
                            None
                        }
                    },
                    Err(e) => {
                        eprintln!("getCoinbaseTx request failed: {}", e);
                        None
                    }
                }
            };

            // 50% of the time, try to submit with nonce=0 (the header hash
            // meets the regtest target ~50% of the time). Reuse the template
            // header's version and timestamp: the node chose them to be
            // contextually valid, whereas our wall clock may not be (e.g.
            // under time manipulation faults).
            if random_range(2) == 0 {
                if let (Some(coinbase), Some(header)) = (&coinbase_data, &template_header) {
                    let mut req = template_client.submit_solution_request();
                    req.get().get_context().unwrap().set_thread(thread.clone());
                    req.get().set_version(header.version.to_consensus() as u32);
                    req.get().set_timestamp(header.time);
                    req.get().set_nonce(0);
                    req.get().set_coinbase(coinbase);
                    match req.send().promise.await {
                        Ok(response) => match response.get() {
                            Ok(result) => {
                                let accepted = result.get_result();
                                let reason = result
                                    .get_reason()
                                    .ok()
                                    .and_then(|r| r.to_str().ok())
                                    .unwrap_or("")
                                    .to_string();
                                let debug = result
                                    .get_debug()
                                    .ok()
                                    .and_then(|d| d.to_str().ok())
                                    .unwrap_or("")
                                    .to_string();
                                antithesis_sdk::assert_sometimes!(
                                    accepted,
                                    "IPC submitSolution sometimes succeeds on regtest",
                                    &serde_json::json!({
                                        "accepted": accepted,
                                        "reason": reason,
                                        "debug": debug,
                                    })
                                );
                                println!(
                                    "submitSolution: accepted={} reason={} debug={}",
                                    accepted, reason, debug
                                );
                            }
                            Err(e) => eprintln!("submitSolution response error: {}", e),
                        },
                        Err(e) => eprintln!("submitSolution request failed: {}", e),
                    }
                }
            }

            // Destroy the template
            {
                let mut req = template_client.destroy_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(_) => println!("Block template destroyed"),
                    Err(e) => eprintln!("destroy request failed: {}", e),
                }
            }
        })
        .await;
}
