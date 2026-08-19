//! With every node on the same tip, the coins visible across all wallets cannot exceed the
//! subsidy that chain has issued.
//!
//! Conditioned on a shared tip rather than run in a fault-free window: nodes on competing
//! branches each hold their own coinbases, so a partition makes this vacuous rather than
//! wrong, and it keeps checking while faults are active.
//!
//! Assumes no two wallets own the same coin — if a driver ever imports one descriptor into
//! two wallets, both count it and this has to be relaxed.

use bitcoin_antithesis_workload::{
    check_money, create_client, get_all_nodes, get_balances, get_blockchain_info,
    sats_to_btc_string, total_subsidy_issued_sats, BlockchainInfo, Client, Money,
};
use serde_json::json;

/// One node's tip, or `None` if it couldn't be read.
fn tip(client: &Client) -> Option<BlockchainInfo> {
    get_blockchain_info(client).ok()
}

/// The tip every node agrees on, if they all reported one and it's the same.
fn shared_tip(tips: &[Option<BlockchainInfo>]) -> Option<(String, u64)> {
    let mut infos = tips.iter().map(|t| t.as_ref());
    let first = infos.next()??;
    for info in infos {
        let info = info?;
        if info.bestblockhash != first.bestblockhash {
            return None;
        }
    }
    Some((first.bestblockhash.clone(), first.blocks))
}

fn main() {
    antithesis_sdk::antithesis_init();

    let nodes = get_all_nodes();
    let clients: Vec<(String, Client)> = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, config)| match create_client(config) {
            Ok(client) => Some((format!("node{}", i + 1), client)),
            Err(e) => {
                eprintln!("[supply_bound] node{} client creation failed: {}", i + 1, e);
                None
            }
        })
        .collect();

    let all_clients = clients.len() == nodes.len();

    let tips_before: Vec<Option<BlockchainInfo>> =
        clients.iter().map(|(_, client)| tip(client)).collect();

    let mut total: i128 = 0;
    let mut observed: Vec<serde_json::Value> = Vec::new();
    let mut unobservable: Vec<&str> = Vec::new();

    for (name, client) in clients.iter() {
        match get_balances(client) {
            Ok(balances) => {
                let mut wallet_total: i128 = 0;
                for (_, value) in balances.mine.fields() {
                    if let Money::Valid(amount) = check_money(value) {
                        wallet_total += amount as i128;
                    }
                }
                total += wallet_total;
                observed.push(json!({ "node": name, "balance_sats": wallet_total }));
            }
            Err(e) => {
                // Fewer coins visible cannot break an upper bound.
                eprintln!("[supply_bound] {} getbalances failed: {}", name, e);
                unobservable.push(name);
            }
        }
    }

    let tips_after: Vec<Option<BlockchainInfo>> =
        clients.iter().map(|(_, client)| tip(client)).collect();

    // Required either side of the balance reads: otherwise the cluster reorged mid-check
    // and the coins counted came from more than one branch.
    let before = shared_tip(&tips_before);
    let after = shared_tip(&tips_after);
    let stable_tip = match (&before, &after) {
        (Some((hash_before, _)), Some((hash_after, height))) if hash_before == hash_after => {
            Some((hash_after.clone(), *height))
        }
        _ => None,
    };

    let evaluated = all_clients && stable_tip.is_some();

    antithesis_sdk::assert_sometimes!(
        evaluated,
        "The cluster supply bound is evaluated with every node on one tip",
        &json!({
            "all_clients": all_clients,
            "tip_before": before.map(|(hash, _)| hash),
            "tip_after": after.map(|(hash, _)| hash),
        })
    );

    if let Some((hash, height)) = stable_tip {
        if !all_clients {
            return;
        }
        let issued = total_subsidy_issued_sats(height);
        antithesis_sdk::assert_always!(
            total <= issued as i128,
            "Coins across all wallets do not exceed the subsidy issued",
            &json!({
                "tip": hash,
                "height": height,
                "total_sats": total,
                "subsidy_issued_sats": issued,
                "total_btc": sats_to_btc_string(total),
                "per_node": observed,
                "unobservable": unobservable,
            })
        );
    }
}
