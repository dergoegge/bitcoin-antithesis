use bitcoin_antithesis_workload::{get_all_ipc_nodes, ipc, random_ipc_node};
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

            // Test isTestChain
            {
                let mut req = mining_client.is_test_chain_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => {
                            let is_test = result.get_result();
                            antithesis_sdk::assert_always!(
                                is_test,
                                "IPC isTestChain returns true on regtest",
                                &serde_json::json!({ "is_test_chain": is_test })
                            );
                        }
                        Err(e) => eprintln!("isTestChain response error: {}", e),
                    },
                    Err(e) => eprintln!("isTestChain request failed: {}", e),
                }
            }

            // Test isInitialBlockDownload
            {
                let mut req = mining_client.is_initial_block_download_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => {
                            let is_ibd = result.get_result();
                            antithesis_sdk::assert_sometimes!(
                                !is_ibd,
                                "IPC isInitialBlockDownload eventually returns false",
                                &serde_json::json!({ "is_ibd": is_ibd })
                            );
                        }
                        Err(e) => eprintln!("isInitialBlockDownload response error: {}", e),
                    },
                    Err(e) => eprintln!("isInitialBlockDownload request failed: {}", e),
                }
            }

            // Test getTip
            {
                let mut req = mining_client.get_tip_request();
                req.get().get_context().unwrap().set_thread(thread.clone());
                match req.send().promise.await {
                    Ok(response) => match response.get() {
                        Ok(result) => {
                            let has_result = result.get_has_result();
                            if has_result {
                                match result.get_result() {
                                    Ok(block_ref) => {
                                        let height = block_ref.get_height();
                                        antithesis_sdk::assert_always!(
                                            height >= 0,
                                            "IPC getTip returns non-negative height",
                                            &serde_json::json!({ "height": height })
                                        );
                                        antithesis_sdk::assert_sometimes_greater_than!(
                                            height,
                                            100,
                                            "IPC getTip height eventually exceeds 100",
                                            &serde_json::json!({ "height": height })
                                        );
                                    }
                                    Err(e) => eprintln!("getTip result error: {}", e),
                                }
                            } else {
                                println!("getTip: no tip available yet");
                            }
                        }
                        Err(e) => eprintln!("getTip response error: {}", e),
                    },
                    Err(e) => eprintln!("getTip request failed: {}", e),
                }
            }
        })
        .await;
}
