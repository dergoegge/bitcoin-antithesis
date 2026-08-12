//! Mines blocks on a random node.
//!
//! When it picks the GUI's node the application mines and its own tip moves;
//! when it picks node1 the tip arrives over the wire and the GUI has to follow
//! it. Both are worth exploring against, and mining on both sides is what
//! produces the competing tips that make the GUI reorg.

use antithesis_sdk::random::random_choice;
use qml_gui_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, create_wallet_client,
    ensure_wallet, get_all_nodes, random_node_pair,
};

fn main() {
    let nodes = get_all_nodes();
    let (node, _) = random_node_pair(&nodes);

    let client = match create_client(node) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[blocks] failed to create client for {}: {}", node.name, e);
            return;
        }
    };
    let wallet = match create_wallet_client(node) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "[blocks] failed to create wallet client for {}: {}",
                node.name, e
            );
            return;
        }
    };

    ensure_wallet(&client, &node.name);

    // Mine to a fresh address so the rewards keep landing in the wallet the GUI
    // displays.
    let address: String = match wallet.call("getnewaddress", &[]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[blocks] failed to get new address on {}: {}", node.name, e);
            return;
        }
    };

    let block_options = [1, 2, 16, 32, 128u64];
    let num_blocks = *random_choice(&block_options).expect("block_options is non-empty");

    match client.call::<Vec<String>>(
        "generatetoaddress",
        &[serde_json::json!(num_blocks), serde_json::json!(address)],
    ) {
        Ok(block_hashes) => {
            println!(
                "[blocks] mined {} blocks on {}",
                block_hashes.len(),
                node.name
            );

            assert_mempool_metrics(&client, "after_mining");
            assert_wallet_metrics(&wallet, "after_mining");
        }
        Err(e) => {
            eprintln!("[blocks] failed to mine on {}: {}", node.name, e);
        }
    }
}
