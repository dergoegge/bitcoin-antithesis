use ir_workload::{
    create_client, get_all_nodes, FullProgramContext, Header, IrBuilderClient, ProgramContext, Txo,
};
use std::thread;
use std::time::Duration;

// P2WSH OP_TRUE: raw scriptPubKey = OP_0 PUSH32 SHA256(OP_TRUE)
// Using raw() at top level with full scriptPubKey
const P2WSH_OP_TRUE_DESC: &str =
    "raw(00204ae81572f06e1b88fd5ced7a1a000945432e83e1551e6f721ee9c00b8cc33260)";

// The scriptPubKey bytes for P2WSH OP_TRUE
const P2WSH_OP_TRUE_SCRIPT_PUBKEY: [u8; 34] = [
    0x00, 0x20, // OP_0 PUSH32
    0x4a, 0xe8, 0x15, 0x72, 0xf0, 0x6e, 0x1b, 0x88, 0xfd, 0x5c, 0xed, 0x7a, 0x1a, 0x00, 0x09, 0x45,
    0x43, 0x2e, 0x83, 0xe1, 0x55, 0x1e, 0x6f, 0x72, 0x1e, 0xe9, 0xc0, 0x0b, 0x8c, 0xc3, 0x32, 0x60,
];

// The witness script for P2WSH OP_TRUE (just OP_TRUE = 0x51)
const OP_TRUE_WITNESS_SCRIPT: [u8; 1] = [0x51];

fn main() {
    antithesis_sdk::antithesis_init();

    println!("Health checker: waiting for all nodes to be ready...");

    let nodes = get_all_nodes();
    let mut all_ready = false;

    while !all_ready {
        all_ready = true;

        for (i, node_config) in nodes.iter().enumerate() {
            let client = match create_client(node_config) {
                Ok(c) => c,
                Err(e) => {
                    println!("node{}: client error ({})", i + 1, e);
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

    // Generate 200 blocks to the p2wsh OP_TRUE descriptor on node1
    let client1 = create_client(&nodes[0]).expect("failed to create client for node1");
    println!("Health checker: generating 200 blocks on node1...");
    let block_hashes: Vec<String> = client1
        .call(
            "generatetodescriptor",
            &[
                serde_json::json!(200),
                serde_json::json!(P2WSH_OP_TRUE_DESC),
            ],
        )
        .expect("failed to generate blocks");
    println!("node1: generated {} blocks", block_hashes.len());

    // Get the tip block hash from node1
    let tip_hash = block_hashes.last().expect("no blocks generated");
    println!("Health checker: tip block hash = {}", tip_hash);

    // Wait for node2 to sync to the tip
    let client2 = create_client(&nodes[1]).expect("failed to create client for node2");
    println!("Health checker: waiting for node2 to sync...");
    let result: serde_json::Value = client2
        .call(
            "waitforblock",
            &[serde_json::json!(tip_hash), serde_json::json!(0)],
        )
        .expect("failed to wait for block");
    let chain_height = result["height"].as_u64().expect("height not found");
    println!("node2: synced to height {}", chain_height);

    // Collect only the tip header
    println!("Health checker: collecting tip header...");
    let tip_block_hash = block_hashes.last().expect("no blocks generated");
    let header_info: serde_json::Value = client1
        .call("getblockheader", &[serde_json::json!(tip_block_hash)])
        .expect("failed to get block header");

    let prev_hash_str = header_info["previousblockhash"]
        .as_str()
        .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000");
    let merkle_root_str = header_info["merkleroot"].as_str().expect("merkleroot");

    let mut prev = [0u8; 32];
    hex::decode_to_slice(prev_hash_str, &mut prev).expect("decode prev");
    prev.reverse(); // Bitcoin uses little-endian internally

    let mut merkle_root = [0u8; 32];
    hex::decode_to_slice(merkle_root_str, &mut merkle_root).expect("decode merkle");
    merkle_root.reverse();

    let tip_header = Header {
        prev,
        merkle_root,
        nonce: header_info["nonce"].as_u64().expect("nonce") as u32,
        bits: u32::from_str_radix(header_info["bits"].as_str().expect("bits"), 16)
            .expect("parse bits"),
        time: header_info["time"].as_u64().expect("time") as u32,
        version: header_info["version"].as_i64().expect("version") as i32,
        height: header_info["height"].as_u64().expect("height") as u32,
    };
    println!(
        "Health checker: collected tip header at height {}",
        tip_header.height
    );
    let tip_time = tip_header.time;
    let headers = vec![tip_header];

    // Collect only spendable TXOs (at least 100 blocks deep)
    // With 200 blocks, only the first 100 blocks have spendable coinbase outputs
    println!("Health checker: collecting spendable txos (100+ confirmations)...");
    let mut txos: Vec<Txo> = Vec::new();
    let num_spendable = block_hashes.len().saturating_sub(100);

    for (i, block_hash) in block_hashes.iter().take(num_spendable).enumerate() {
        // Get block with transactions to extract coinbase output
        let block: serde_json::Value = client1
            .call(
                "getblock",
                &[serde_json::json!(block_hash), serde_json::json!(2)],
            )
            .expect("failed to get block");

        let coinbase_tx = &block["tx"][0];
        let txid_str = coinbase_tx["txid"].as_str().expect("txid");
        let mut txid = [0u8; 32];
        hex::decode_to_slice(txid_str, &mut txid).expect("decode txid");
        txid.reverse(); // Bitcoin uses little-endian

        // The coinbase output at index 0 is the P2WSH OP_TRUE output
        let vout = &coinbase_tx["vout"][0];
        let value_btc = vout["value"].as_f64().expect("value");
        let value_sats = (value_btc * 100_000_000.0) as u64;

        let txo = Txo {
            outpoint: (txid, 0),
            value: value_sats,
            script_pubkey: P2WSH_OP_TRUE_SCRIPT_PUBKEY.to_vec(),
            spending_script_sig: vec![], // P2WSH uses witness, no scriptSig
            spending_witness: vec![OP_TRUE_WITNESS_SCRIPT.to_vec()],
        };
        txos.push(txo);

        if (i + 1) % 50 == 0 {
            println!("  collected {} spendable txos", i + 1);
        }
    }

    println!(
        "Health checker: collected {} header(s) and {} spendable txos",
        headers.len(),
        txos.len()
    );

    // Build the FullProgramContext
    let full_context = FullProgramContext {
        context: ProgramContext {
            num_nodes: nodes.len(),
            num_connections: 8, // Default connections between harness and nodes
            timestamp: tip_time as u64,
        },
        txos,
        headers,
    };

    // Wait for ir-builder to be ready and initialize it with the context
    println!("Health checker: initializing ir-builder with context...");
    let ir_client = IrBuilderClient::from_env();
    loop {
        match ir_client.init(&full_context) {
            Ok(response) if response.success => {
                println!("ir-builder: initialized (result: {:?})", response.result);
                break;
            }
            Ok(response) => {
                println!("ir-builder: init failed (error: {:?})", response.error);
            }
            Err(e) => {
                println!("ir-builder: not ready ({})", e);
            }
        }
        thread::sleep(Duration::from_secs(1));
    }

    // Sleep until tip time + 2 mins to ensure new blocks (generated by IR) have good timestamps
    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_secs();
    let target_time = tip_time as u64 + 120;
    if current_time < target_time {
        let sleep_duration = target_time - current_time;
        println!(
            "Health checker: sleeping {} seconds to reach target time {}...",
            sleep_duration, target_time
        );
        thread::sleep(Duration::from_secs(sleep_duration));
    }

    // Signal to Antithesis that setup is complete
    antithesis_sdk::lifecycle::setup_complete(&serde_json::json!({
        "message": "Bitcoin cluster with ir-builder is ready",
        "node_count": nodes.len(),
        "chain_height": chain_height,
        "txo_count": full_context.txos.len(),
        "header_count": full_context.headers.len()
    }));

    println!("Health checker: setup_complete signaled, exiting");
}
