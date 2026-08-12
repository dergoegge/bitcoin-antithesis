//! Brings the chain into a state worth looking at before the GUI is explored.
//!
//! Same setup as the initial RPC workload: a wallet on every node, coinbase
//! rewards spread across them, and enough blocks on top for those rewards to be
//! spendable. The difference here is that one of those nodes is the GUI's, so
//! the application starts out with a funded wallet, a transaction history and a
//! peer, rather than the empty chain a fresh regtest datadir would give it.

use qml_gui_workload::{create_client, create_wallet_client, ensure_wallet, get_all_nodes};
use std::thread;
use std::time::Duration;

/// Coinbase rewards mined to each node's own address.
const BLOCKS_PER_NODE: u64 = 10;
/// Blocks mined on top so that every reward above is past maturity.
const MATURITY_BLOCKS: u64 = 100;

fn main() {
    antithesis_sdk::antithesis_init();

    println!("Health checker: waiting for all nodes to be ready...");

    let nodes = get_all_nodes();
    let mut all_ready = false;

    while !all_ready {
        all_ready = true;

        for node in &nodes {
            let client = match create_client(node) {
                Ok(c) => c,
                Err(e) => {
                    println!("{}: failed to create client ({})", node.name, e);
                    all_ready = false;
                    continue;
                }
            };
            match client.call::<serde_json::Value>("getblockchaininfo", &[]) {
                Ok(info) => {
                    println!("{}: ready (blocks: {})", node.name, info["blocks"]);
                }
                Err(e) => {
                    println!("{}: not ready ({})", node.name, e);
                    all_ready = false;
                }
            }
        }

        if !all_ready {
            thread::sleep(Duration::from_secs(1));
        }
    }

    println!("Health checker: all nodes are ready!");

    // Create or load a wallet on each node and get a new address. The GUI picks
    // up the wallet its own node loads, which is what puts a balance and an
    // activity list on screen.
    println!("Health checker: creating/loading wallets and getting addresses...");
    let mut addresses: Vec<String> = Vec::new();
    for node in &nodes {
        let client = match create_client(node) {
            Ok(c) => c,
            Err(e) => panic!(
                "{}: failed to create client for wallet setup: {}",
                node.name, e
            ),
        };
        ensure_wallet(&client, &node.name);

        let wallet =
            create_wallet_client(node).expect("failed to create wallet client for wallet setup");
        let address: String = wallet
            .call("getnewaddress", &[])
            .expect("failed to get address");
        println!("{}: address = {}", node.name, address);
        addresses.push(address);
    }

    // Generate the initial chain on node1, distributing rewards across all node
    // addresses. Mining on node1 rather than on the GUI's node means the GUI
    // receives its coins over the wire, the way a real wallet does.
    println!("Health checker: generating initial chain on node1...");
    let client = create_client(&nodes[0]).expect("failed to create client for node1");

    for (node, address) in nodes.iter().zip(&addresses) {
        match client.call::<Vec<String>>(
            "generatetoaddress",
            &[
                serde_json::json!(BLOCKS_PER_NODE),
                serde_json::json!(address),
            ],
        ) {
            Ok(blocks) => {
                println!(
                    "node1: generated {} blocks to {}'s address",
                    blocks.len(),
                    node.name
                );
            }
            Err(e) => {
                println!(
                    "node1: failed to generate blocks to {}'s address: {}",
                    node.name, e
                );
            }
        }
    }

    println!("Health checker: generating maturity blocks on node1...");
    match client.call::<Vec<String>>(
        "generatetoaddress",
        &[
            serde_json::json!(MATURITY_BLOCKS),
            serde_json::json!(&addresses[0]),
        ],
    ) {
        Ok(blocks) => println!("node1: generated {} maturity blocks", blocks.len()),
        Err(e) => println!("node1: failed to generate maturity blocks: {}", e),
    }

    // Get the target block count from node1
    let target_info: serde_json::Value = client
        .call("getblockchaininfo", &[])
        .expect("failed to get blockchain info");
    let target_blocks = target_info["blocks"].as_u64().expect("blocks not found");
    println!("Health checker: target chain height is {}", target_blocks);

    // Wait for all nodes to sync to the same height. For the GUI this is also
    // what makes its wallet balance non-zero.
    println!("Health checker: waiting for all nodes to sync...");
    loop {
        let mut all_synced = true;

        for node in &nodes {
            let client = match create_client(node) {
                Ok(c) => c,
                Err(e) => {
                    println!("{}: failed to create client ({})", node.name, e);
                    all_synced = false;
                    continue;
                }
            };
            match client.call::<serde_json::Value>("getblockchaininfo", &[]) {
                Ok(info) => {
                    let blocks = info["blocks"].as_u64().unwrap_or(0);
                    if blocks < target_blocks {
                        println!("{}: syncing ({}/{})", node.name, blocks, target_blocks);
                        all_synced = false;
                    } else {
                        println!("{}: synced ({})", node.name, blocks);
                    }
                }
                Err(e) => {
                    println!("{}: error checking sync status: {}", node.name, e);
                    all_synced = false;
                }
            }
        }

        if all_synced {
            break;
        }

        thread::sleep(Duration::from_secs(1));
    }

    println!("Health checker: all nodes synced!");

    // Report what the GUI is starting from, so a failed run can be read
    // without guessing at the initial state.
    for node in &nodes {
        if let Ok(wallet) = create_wallet_client(node) {
            match wallet.call::<serde_json::Value>("getbalances", &[]) {
                Ok(balances) => println!("{}: balances {}", node.name, balances),
                Err(e) => println!("{}: failed to read balances: {}", node.name, e),
            }
        }
    }

    // Signal to Antithesis that setup is complete
    antithesis_sdk::lifecycle::setup_complete(&serde_json::json!({
        "message": "Chain is synced and the GUI's wallet is funded",
        "node_count": nodes.len(),
        "chain_height": target_blocks
    }));

    println!("Health checker: setup_complete signaled, exiting");
}
