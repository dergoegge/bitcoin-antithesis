use bitcoin_antithesis_workload::{get_all_ipc_nodes, ipc, random_ipc_node, random_range};
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

            let echo_client = match ipc::make_echo(&init_client, &thread).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Failed to create echo client: {}", e);
                    return;
                }
            };

            // Generate a random test string
            let random_val = random_range(1_000_000);
            let test_string = format!("antithesis_echo_test_{}", random_val);

            let mut req = echo_client.echo_request();
            req.get().get_context().unwrap().set_thread(thread.clone());
            req.get().set_echo(&test_string);

            match req.send().promise.await {
                Ok(response) => match response.get() {
                    Ok(result) => match result.get_result() {
                        Ok(text) => {
                            let echoed = match text.to_str() {
                                Ok(s) => s.to_string(),
                                Err(e) => {
                                    eprintln!("Echo result is not valid UTF-8: {}", e);
                                    return;
                                }
                            };
                            antithesis_sdk::assert_always!(
                                echoed == test_string,
                                "IPC Echo response matches input",
                                &serde_json::json!({
                                    "sent": test_string,
                                    "received": echoed
                                })
                            );
                            println!(
                                "Echo test passed: sent='{}', got='{}'",
                                test_string, echoed
                            );
                        }
                        Err(e) => eprintln!("Failed to get echo result: {}", e),
                    },
                    Err(e) => eprintln!("Failed to get echo response: {}", e),
                },
                Err(e) => eprintln!("Echo request failed: {}", e),
            }
        })
        .await;
}
