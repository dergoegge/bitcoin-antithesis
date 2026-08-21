//! Every money amount a wallet reports is in range, and no wallet holds more than the
//! subsidy issued so far.
//!
//! Each assertion is judged within a single RPC response: nothing keeps two calls
//! consistent, since a wallet transaction entering the mempool changes the wallet without
//! moving the tip.

use bitcoin_antithesis_workload::{
    check_money, create_client, get_all_nodes, get_balances, get_blockchain_info,
    sats_to_btc_string, total_subsidy_issued_sats, BalanceBuckets, Balances, Client, Money,
};
use serde_json::{json, Value};

fn main() {
    antithesis_sdk::antithesis_init();

    for (i, config) in get_all_nodes().iter().enumerate() {
        let name = format!("node{}", i + 1);
        match create_client(config) {
            Ok(client) => check_node(&name, &client),
            Err(e) => eprintln!("[wallet_amounts] {} client creation failed: {}", name, e),
        }
    }
}

fn check_node(node: &str, client: &Client) {
    let height_before = get_blockchain_info(client).map(|info| info.blocks).ok();

    let balances = get_balances(client);
    let unspent = client.call::<Vec<Value>>("listunspent", &[json!(0), json!(9_999_999)]);
    // minconf=0 includes inactive transactions, which is where GetReceived() accumulates
    // every retained replacement.
    let received = client.call::<Vec<Value>>("listreceivedbyaddress", &[json!(0), json!(true)]);
    let transactions = client.call::<Vec<Value>>("listtransactions", &[json!("*"), json!(1000)]);

    let height_after = get_blockchain_info(client).map(|info| info.blocks).ok();

    // Taking the largest of the heights either side means a concurrent block or
    // invalidateblock can only loosen the bound.
    let processed = balances
        .as_ref()
        .ok()
        .and_then(|b| b.lastprocessedblock.as_ref())
        .map(|b| b.height);
    let bound = [height_before, height_after, processed]
        .into_iter()
        .flatten()
        .max()
        .map(total_subsidy_issued_sats);

    match balances {
        Ok(balances) => check_balances(node, &balances, bound),
        Err(e) => eprintln!("[wallet_amounts] {} getbalances failed: {}", node, e),
    }
    match unspent {
        Ok(utxos) => check_unspent(node, &utxos, bound),
        Err(e) => eprintln!("[wallet_amounts] {} listunspent failed: {}", node, e),
    }
    match received {
        Ok(entries) => check_received(node, &entries),
        Err(e) => eprintln!(
            "[wallet_amounts] {} listreceivedbyaddress failed: {}",
            node, e
        ),
    }
    match transactions {
        Ok(txs) => check_transactions(node, &txs),
        Err(e) => eprintln!("[wallet_amounts] {} listtransactions failed: {}", node, e),
    }
}

/// Describe an invalid amount for the assertion details.
fn offender(what: &str, value: &Value, money: Money) -> Value {
    json!({
        "field": what,
        "value": value,
        "reason": match money {
            Money::NotANumber => "not a number",
            Money::OutOfRange(_) => "outside [-MAX_MONEY, MAX_MONEY] or not finite",
            Money::SubSatoshi(_) => "not a whole number of satoshis",
            Money::Valid(_) => "valid",
        },
    })
}

fn check_balances(node: &str, balances: &Balances, bound: Option<i64>) {
    let mut bad = Vec::new();
    let mut total: i128 = 0;

    for (name, value) in balances.mine.fields() {
        match check_money(value) {
            Money::Valid(amount) => {
                total += amount as i128;
                if amount < 0 && !BalanceBuckets::is_signed(name) {
                    bad.push(json!({ "field": name, "value": value, "reason": "negative" }));
                }
            }
            // `used` is only present on avoid_reuse wallets, `nonmempool` only on
            // branches that report it.
            Money::NotANumber if value.is_null() => {}
            other => bad.push(offender(name, value, other)),
        }
    }

    antithesis_sdk::assert_always!(
        bad.is_empty(),
        "Wallet balance buckets are within MoneyRange",
        &json!({ "node": node, "offenders": bad })
    );

    if let Some(bound) = bound {
        antithesis_sdk::assert_always!(
            total <= bound as i128,
            "Wallet balance does not exceed the subsidy issued",
            &json!({
                "node": node,
                "total_sats": total,
                "subsidy_issued_sats": bound,
                "total_btc": sats_to_btc_string(total),
            })
        );
    }
}

fn check_unspent(node: &str, utxos: &[Value], bound: Option<i64>) {
    let mut bad = Vec::new();
    let mut total: i128 = 0;

    for utxo in utxos {
        match check_money(&utxo["amount"]) {
            Money::Valid(amount) => {
                total += amount as i128;
                // Zero-value outputs are consensus-valid.
                if amount < 0 {
                    bad.push(json!({
                        "field": "amount", "value": utxo["amount"], "reason": "negative",
                        "outpoint": format!("{}:{}", utxo["txid"], utxo["vout"]),
                    }));
                }
            }
            other => {
                let mut entry = offender("amount", &utxo["amount"], other);
                entry["outpoint"] = json!(format!("{}:{}", utxo["txid"], utxo["vout"]));
                bad.push(entry);
            }
        }
    }

    antithesis_sdk::assert_always!(
        bad.is_empty(),
        "Every wallet UTXO amount is within MoneyRange",
        &json!({ "node": node, "utxos": utxos.len(), "offenders": bad })
    );

    if let Some(bound) = bound {
        antithesis_sdk::assert_always!(
            total <= bound as i128,
            "Wallet UTXO total does not exceed the subsidy issued",
            &json!({
                "node": node,
                "total_sats": total,
                "subsidy_issued_sats": bound,
                "total_btc": sats_to_btc_string(total),
            })
        );
    }
}

fn check_received(node: &str, entries: &[Value]) {
    let mut bad = Vec::new();

    for entry in entries {
        match check_money(&entry["amount"]) {
            Money::Valid(amount) if amount >= 0 => {}
            Money::Valid(amount) => bad.push(json!({
                "address": entry["address"], "value": amount, "reason": "negative",
            })),
            other => {
                let mut offender = offender("amount", &entry["amount"], other);
                offender["address"] = entry["address"].clone();
                bad.push(offender);
            }
        }
    }

    antithesis_sdk::assert_always!(
        bad.is_empty(),
        "Every received-by-address total is within MoneyRange",
        &json!({ "node": node, "addresses": entries.len(), "offenders": bad })
    );
}

fn check_transactions(node: &str, txs: &[Value]) {
    let mut bad = Vec::new();
    let mut bad_send_signs = Vec::new();

    for tx in txs {
        let category = tx["category"].as_str().unwrap_or("");
        let amount = check_money(&tx["amount"]);
        if let Money::Valid(value) = amount {
            // `amount` is documented negative for send, positive otherwise.
            match category {
                "send" if value > 0 => bad_send_signs.push(json!({
                    "txid": tx["txid"], "field": "amount", "value": value,
                })),
                "receive" | "generate" | "immature" if value < 0 => bad_send_signs.push(json!({
                    "txid": tx["txid"], "category": category,
                    "field": "amount", "value": value,
                })),
                _ => {}
            }
        } else {
            let mut entry = offender("amount", &tx["amount"], amount);
            entry["txid"] = tx["txid"].clone();
            bad.push(entry);
        }

        // `fee` is documented negative, and only present for send.
        if let Some(fee) = tx.get("fee") {
            match check_money(fee) {
                Money::Valid(value) if value <= 0 => {}
                Money::Valid(value) => bad_send_signs.push(json!({
                    "txid": tx["txid"], "field": "fee", "value": value,
                })),
                other => {
                    let mut entry = offender("fee", fee, other);
                    entry["txid"] = tx["txid"].clone();
                    bad.push(entry);
                }
            }
        }
    }

    antithesis_sdk::assert_always!(
        bad.is_empty(),
        "Every wallet transaction amount and fee is within MAX_MONEY",
        &json!({ "node": node, "transactions": txs.len(), "offenders": bad })
    );

    antithesis_sdk::assert_always!(
        bad_send_signs.is_empty(),
        "Wallet transaction amounts and fees have the documented sign",
        &json!({ "node": node, "transactions": txs.len(), "offenders": bad_send_signs })
    );
}
