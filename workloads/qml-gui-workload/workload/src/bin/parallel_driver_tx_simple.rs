//! Sends a simple transfer between the two nodes.
//!
//! The sender and receiver are picked at random, so the GUI sees both outgoing
//! and incoming payments: a transaction it created itself, and one that arrives
//! from a peer and has to show up in its activity list unprompted.

use qml_gui_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, create_wallet_client,
    ensure_wallet, get_all_nodes, random_amount, random_node_pair, Client, NodeConfig,
};
use serde_json::json;

/// The node's own client, and one addressed at the workload's wallet on it.
fn clients_for(node: &NodeConfig) -> Option<(Client, Client)> {
    let client = create_client(node)
        .inspect_err(|e| eprintln!("[tx_simple] no client for {}: {}", node.name, e))
        .ok()?;
    let wallet = create_wallet_client(node)
        .inspect_err(|e| eprintln!("[tx_simple] no wallet client for {}: {}", node.name, e))
        .ok()?;
    ensure_wallet(&client, &node.name);
    Some((client, wallet))
}

fn main() {
    let nodes = get_all_nodes();
    let (sender, receiver) = random_node_pair(&nodes);

    let Some((sender_client, sender_wallet)) = clients_for(sender) else {
        return;
    };
    let Some((_, receiver_wallet)) = clients_for(receiver) else {
        return;
    };

    let address: String = match receiver_wallet.call("getnewaddress", &[]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!(
                "[tx_simple] failed to get new address on {}: {}",
                receiver.name, e
            );
            return;
        }
    };

    let amount = random_amount();

    match sender_wallet.call::<String>("sendtoaddress", &[json!(address), json!(amount)]) {
        Ok(txid) => {
            println!(
                "[tx_simple] {} sent {} BTC to {}: {}",
                sender.name, amount, receiver.name, txid
            );

            assert_mempool_metrics(&sender_client, "after_simple_send");
            assert_wallet_metrics(&sender_wallet, "after_simple_send");
        }
        // Insufficient funds is expected while the wallet's coins are tied up in
        // unconfirmed change, so this is not treated as an error.
        Err(e) => eprintln!("[tx_simple] {} failed to send: {}", sender.name, e),
    }
}
