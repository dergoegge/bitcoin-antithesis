use bitcoin_antithesis_workload::{
    assert_mempool_metrics, assert_wallet_metrics, create_client, get_all_nodes, random_amount,
    random_node, random_range,
};
use serde_json::json;

fn main() {
    let nodes = get_all_nodes();
    let node_config = random_node(&nodes);
    let client = match create_client(node_config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[rbf] Failed to create client: {}", e);
            return;
        }
    };

    let address: String = match client.call("getnewaddress", &[]) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("[rbf] Failed to get address: {}", e);
            return;
        }
    };

    let amount = random_amount();

    // Use send RPC with options for RBF
    let outputs = json!([{address.clone(): amount}]);
    let options = json!({
        "fee_rate": 1,
        "replaceable": true,
    });

    #[derive(serde::Deserialize)]
    struct SendResult {
        txid: String,
    }

    let result: SendResult = match client.call::<SendResult>(
        "send",
        &[outputs, json!(null), json!(null), json!(null), options],
    ) {
        Ok(r) => {
            println!("[rbf] Created RBF transaction: {}", r.txid);
            r
        }
        Err(e) => {
            eprintln!("[rbf] Failed to create RBF tx: {}", e);
            return;
        }
    };

    // 50% chance to bump the fee
    if random_range(2) == 0 {
        #[derive(serde::Deserialize)]
        struct BumpResult {
            txid: String,
        }
        match client.call::<BumpResult>("bumpfee", &[json!(result.txid)]) {
            Ok(bumped) => println!("[rbf] Bumped fee, new txid: {}", bumped.txid),
            Err(e) => eprintln!("[rbf] Failed to bump fee (may already be confirmed): {}", e),
        }
    }

    assert_mempool_metrics(&client, "after_rbf");
    assert_wallet_metrics(&client, "after_rbf");
}
