use bitcoin_antithesis_workload::{create_client, get_all_nodes, random_node};

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[gettxoutsetinfo] Failed to create client: {}", e);
            return;
        }
    };

    match client.call::<serde_json::Value>("gettxoutsetinfo", &[]) {
        Ok(info) => {
            println!(
                "[gettxoutsetinfo] txouts: {}, hash: {}",
                info["txouts"], info["hash_serialized_2"]
            );
        }
        Err(e) => {
            eprintln!("[gettxoutsetinfo] Failed: {}", e);
        }
    }
}
