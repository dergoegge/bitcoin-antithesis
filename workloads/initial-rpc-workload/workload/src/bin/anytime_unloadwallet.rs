use bitcoin_antithesis_workload::{create_client, get_all_nodes, random_node};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[unloadwallet] Failed to create client: {}", e);
            return;
        }
    };

    match client.call::<serde_json::Value>("unloadwallet", &[json!("default")]) {
        Ok(_) => {
            println!("[unloadwallet] Unloaded default wallet");
        }
        Err(e) => {
            eprintln!("[unloadwallet] Failed to unload default wallet: {}", e);
        }
    }
}
