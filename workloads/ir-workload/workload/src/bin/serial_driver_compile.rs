use ir_workload::IrBuilderClient;

fn main() {
    let ir_client = IrBuilderClient::from_env();

    match ir_client.compile(serde_json::json!({})) {
        Ok(response) => {
            antithesis_sdk::assert_sometimes!(
                response.success,
                "compile succeeded",
                &serde_json::json!({ "result": response.result })
            );
        }
        Err(_) => {}
    }
}
