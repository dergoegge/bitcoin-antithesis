use bitcoin_antithesis_workload::{create_client, get_all_nodes, random_node};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[getblocktemplate] Failed to create client: {}", e);
            return;
        }
    };

    let template_request = json!({
        "rules": ["segwit"]
    });

    match client.call::<serde_json::Value>("getblocktemplate", &[template_request]) {
        Ok(template) => {
            println!(
                "[getblocktemplate] height: {}, txs: {}, previousblockhash: {}",
                template["height"],
                template["transactions"]
                    .as_array()
                    .map(|t| t.len())
                    .unwrap_or(0),
                template["previousblockhash"]
            );
        }
        Err(e) => {
            eprintln!("[getblocktemplate] Failed: {}", e);
        }
    }
}
