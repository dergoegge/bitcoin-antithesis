use bitcoin_antithesis_workload::{create_client, get_all_nodes};
use std::thread;
use std::time::Duration;

fn main() {
    println!("Health checker: waiting for all nodes to be ready...");

    let nodes = get_all_nodes();
    let mut all_ready = false;

    while !all_ready {
        all_ready = true;

        for (i, node_config) in nodes.iter().enumerate() {
            let client = match create_client(node_config) {
                Ok(c) => c,
                Err(e) => {
                    println!("node{}: failed to create client ({})", i + 1, e);
                    all_ready = false;
                    continue;
                }
            };
            match client.call::<serde_json::Value>("getblockchaininfo", &[]) {
                Ok(info) => {
                    println!("node{}: ready (blocks: {})", i + 1, info["blocks"]);
                }
                Err(e) => {
                    println!("node{}: not ready ({})", i + 1, e);
                    all_ready = false;
                }
            }
        }

        if !all_ready {
            thread::sleep(Duration::from_secs(1));
        }
    }

    println!("Health checker: all nodes are ready!");

    // Create or load a wallet on each node and get a new address
    println!("Health checker: creating/loading wallets and getting addresses...");
    let mut addresses: Vec<String> = Vec::new();
    for (i, node_config) in nodes.iter().enumerate() {
        let client = match create_client(node_config) {
            Ok(c) => c,
            Err(e) => {
                panic!(
                    "node{}: failed to create client for wallet setup: {}",
                    i + 1,
                    e
                );
            }
        };
        match client.call::<serde_json::Value>("createwallet", &[serde_json::json!("default")]) {
            Ok(_) => {
                println!("node{}: wallet created", i + 1);
            }
            Err(_) => {
                // Wallet may already exist, try to load it
                match client
                    .call::<serde_json::Value>("loadwallet", &[serde_json::json!("default")])
                {
                    Ok(_) => {
                        println!("node{}: wallet loaded", i + 1);
                    }
                    Err(e) => {
                        println!("node{}: wallet load failed: {}", i + 1, e);
                    }
                }
            }
        }

        let address: String = client
            .call("getnewaddress", &[])
            .expect("failed to get address");
        println!("node{}: address = {}", i + 1, address);
        addresses.push(address);
    }

    // Generate initial chain on node1, distributing rewards across all node addresses
    println!("Health checker: generating initial chain on node1...");
    let client = create_client(&nodes[0]).expect("failed to create client for node1");

    // Generate 10 blocks to each node's address (30 total, coins not yet spendable)
    for (i, address) in addresses.iter().enumerate() {
        match client.call::<Vec<String>>(
            "generatetoaddress",
            &[serde_json::json!(10), serde_json::json!(address)],
        ) {
            Ok(blocks) => {
                println!(
                    "node1: generated {} blocks to node{}'s address",
                    blocks.len(),
                    i + 1
                );
            }
            Err(e) => {
                println!(
                    "node1: failed to generate blocks to node{}'s address: {}",
                    i + 1,
                    e
                );
            }
        }
    }

    // Generate 100 more blocks to make the first batch spendable
    println!("Health checker: generating maturity blocks on node1...");
    let address = &addresses[0];
    match client.call::<Vec<String>>(
        "generatetoaddress",
        &[serde_json::json!(100), serde_json::json!(address)],
    ) {
        Ok(blocks) => {
            println!("node1: generated {} maturity blocks", blocks.len());
        }
        Err(e) => {
            println!("node1: failed to generate maturity blocks: {}", e);
        }
    }

    // Get the target block count from node1
    let target_info: serde_json::Value = client
        .call("getblockchaininfo", &[])
        .expect("failed to get blockchain info");
    let target_blocks = target_info["blocks"].as_u64().expect("blocks not found");
    println!("Health checker: target chain height is {}", target_blocks);

    // Wait for all nodes to sync to the same height
    println!("Health checker: waiting for all nodes to sync...");
    loop {
        let mut all_synced = true;

        for (i, node_config) in nodes.iter().enumerate() {
            let client = match create_client(node_config) {
                Ok(c) => c,
                Err(e) => {
                    println!("node{}: failed to create client ({})", i + 1, e);
                    all_synced = false;
                    continue;
                }
            };
            match client.call::<serde_json::Value>("getblockchaininfo", &[]) {
                Ok(info) => {
                    let blocks = info["blocks"].as_u64().unwrap_or(0);
                    if blocks < target_blocks {
                        println!("node{}: syncing ({}/{})", i + 1, blocks, target_blocks);
                        all_synced = false;
                    } else {
                        println!("node{}: synced ({})", i + 1, blocks);
                    }
                }
                Err(e) => {
                    println!("node{}: error checking sync status: {}", i + 1, e);
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

    // Call gettxoutsetinfo on all nodes to compute UTXO set
    println!("Health checker: computing UTXO set on all nodes...");
    for (i, node_config) in nodes.iter().enumerate() {
        let client = match create_client(node_config) {
            Ok(c) => c,
            Err(e) => {
                println!(
                    "node{}: failed to create client for gettxoutsetinfo: {}",
                    i + 1,
                    e
                );
                continue;
            }
        };
        match client.call::<serde_json::Value>("gettxoutsetinfo", &[]) {
            Ok(info) => {
                println!(
                    "node{}: txoutsetinfo (txouts: {}, hash: {})",
                    i + 1,
                    info["txouts"],
                    info["hash_serialized_2"]
                );
            }
            Err(e) => {
                println!("node{}: gettxoutsetinfo failed: {}", i + 1, e);
            }
        }
    }

    // Signal to Antithesis that setup is complete
    antithesis_sdk::lifecycle::setup_complete(&serde_json::json!({
        "message": "Bitcoin cluster is healthy and synced",
        "node_count": nodes.len(),
        "chain_height": target_blocks
    }));

    println!("Health checker: setup_complete signaled, exiting");
}
