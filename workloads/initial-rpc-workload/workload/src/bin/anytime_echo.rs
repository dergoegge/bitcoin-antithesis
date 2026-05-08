use bitcoin_antithesis_workload::{create_client, get_all_nodes, random_node};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[echo] Failed to create client: {}", e);
            return;
        }
    };

    let payload_len = (antithesis_sdk::random::get_random() as usize) % 1_999_000;
    let payload = "a".repeat(payload_len);

    let response = client.call::<serde_json::Value>("echo", &[json!(payload)]).unwrap();
    let echoed = response.as_array().unwrap().first().unwrap();
    antithesis_sdk::assert_always!(
        *echoed == json!(payload),
        "echo RPC response matches input",
        &serde_json::json!({
            "input": payload,
            "output": echoed
        })
    );
    println!("[echo] RPC response matched input ({} chars)", payload_len);
}
