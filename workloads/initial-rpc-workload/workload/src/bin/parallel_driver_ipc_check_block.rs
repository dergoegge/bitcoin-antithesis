use bitcoin::consensus::encode::{deserialize, serialize};
use bitcoin::Block;
use bitcoin_antithesis_workload::{get_all_ipc_nodes, ipc, random_ipc_node};
use bitcoin_capnp_types::mining_capnp::{block_check_options, block_create_options};
use tokio::task::LocalSet;

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

            // Create a new block template to get a serialized block
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

            // Get the serialized block
            let block_data = {
                let mut req = template_client.get_block_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => match result.get_result() {
                            Ok(data) => data.to_vec(),
                            Err(e) => {
                                eprintln!("getBlock result error: {}", e);
                                return;
                            }
                        },
                        Err(e) => {
                            eprintln!("getBlock response error: {}", e);
                            return;
                        }
                    },
                    Err(e) => {
                        eprintln!("getBlock request failed: {}", e);
                        return;
                    }
                }
            };

            // The node leaves the template header's merkle root unset: IPC
            // mining clients are expected to finalize the coinbase and compute
            // the merkle root themselves (see getCoinbaseMerklePath /
            // AddMerkleRootAndCoinbase). checkBlock is called with
            // check_merkle_root=true below, so fill it in first.
            let mut block: Block = match deserialize(&block_data) {
                Ok(b) => b,
                Err(e) => {
                    antithesis_sdk::assert_unreachable!(
                        "Failed to deserialize template block",
                        &serde_json::json!({ "error": e.to_string() })
                    );
                    return;
                }
            };
            let Some(merkle_root) = block.compute_merkle_root() else {
                antithesis_sdk::assert_unreachable!("Template block has no transactions");
                return;
            };

            block.header.merkle_root = merkle_root;
            let block_data = serialize(&block);

            println!(
                "Got block of {} bytes for checkBlock testing",
                block_data.len()
            );

            // checkBlock with check_merkle_root=true, check_pow=false
            // This should always succeed since the block is well-formed
            {
                let mut req = mining_client.check_block_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                req.get().set_block(&block_data);
                {
                    let mut options: block_check_options::Builder = req.get().init_options();
                    options.set_check_merkle_root(true);
                    options.set_check_pow(false);
                }
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => {
                            let valid = result.get_result();

                            if !valid {
                                if let Ok(reason) = result.get_reason() {
                                    eprintln!("  reason: {}", reason.to_str().unwrap_or("?"));
                                }
                                if let Ok(debug) = result.get_debug() {
                                    eprintln!("  debug: {}", debug.to_str().unwrap_or("?"));
                                }
                            }
                        }
                        Err(e) => eprintln!("checkBlock response error: {}", e),
                    },
                    Err(e) => eprintln!("checkBlock request failed: {}", e),
                }
            }

            // checkBlock with check_merkle_root=true, check_pow=true
            // On regtest with low difficulty, this sometimes passes
            {
                let mut req = mining_client.check_block_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                req.get().set_block(&block_data);
                {
                    let mut options: block_check_options::Builder = req.get().init_options();
                    options.set_check_merkle_root(true);
                    options.set_check_pow(true);
                }
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => {
                            let valid = result.get_result();
                            antithesis_sdk::assert_sometimes!(
                                valid,
                                "IPC checkBlock(merkle=true, pow=true) sometimes passes on regtest",
                                &serde_json::json!({ "valid": valid })
                            );
                        }
                        Err(e) => eprintln!("checkBlock(pow=true) response error: {}", e),
                    },
                    Err(e) => eprintln!("checkBlock(pow=true) request failed: {}", e),
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
