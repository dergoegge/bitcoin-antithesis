//! Chain-side helpers for the QML GUI workload.
//!
//! The environment is two nodes: `node1`, a plain bitcoind, and `gui`, the node
//! that runs inside `bitcoin-core-app`. Both speak the same RPC interface, so
//! the drivers treat them as interchangeable and the GUI ends up on both sides
//! of the traffic they generate.

use std::env;

use jsonrpc::simple_http::SimpleHttpTransport;
use jsonrpc::Client as JsonRpcClient;
use serde::Deserialize;
use serde_json::value::RawValue;

/// Subset of the getmempoolinfo RPC response
#[derive(Debug, Deserialize, Clone)]
pub struct MempoolInfo {
    pub size: u64,
    pub bytes: u64,
    pub usage: u64,
    pub maxmempool: u64,
    pub mempoolminfee: f64,
}

/// Subset of the getwalletinfo RPC response
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct WalletInfo {
    pub walletname: String,
    pub txcount: u64,
}

/// Bitcoin RPC client wrapper
pub struct Client {
    inner: JsonRpcClient,
}

impl Client {
    pub fn new(url: &str, user: &str, password: &str) -> Result<Self, jsonrpc::simple_http::Error> {
        let transport = SimpleHttpTransport::builder()
            .url(url)?
            .auth(user, Some(password))
            .build();

        Ok(Self {
            inner: JsonRpcClient::with_transport(transport),
        })
    }

    pub fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<T, jsonrpc::Error> {
        let params = if args.is_empty() {
            None
        } else {
            let serialized = serde_json::to_string(args).expect("failed to serialize args");
            Some(RawValue::from_string(serialized).expect("failed to create RawValue"))
        };
        let request = self.inner.build_request(method, params.as_deref());
        let response = self.inner.send_request(request)?;
        response.result()
    }
}

/// Node configuration from environment variables
pub struct NodeConfig {
    /// Lower case name of the node, used for logging.
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
}

impl NodeConfig {
    pub fn from_env(node_name: &str) -> Self {
        let env_var = format!("{}_RPC_URL", node_name.to_uppercase());
        let url = env::var(&env_var)
            .unwrap_or_else(|_| panic!("Missing environment variable: {}", env_var));

        // Parse URL like http://user:password@host:port
        let url = url.trim_start_matches("http://");
        let (auth, host_port) = url.split_once('@').expect("URL must contain @");
        let (user, password) = auth.split_once(':').expect("Auth must be user:password");
        let (host, port) = host_port.split_once(':').expect("Host must be host:port");

        Self {
            name: node_name.to_lowercase(),
            host: host.to_string(),
            port: port.parse().expect("Invalid port"),
            user: user.to_string(),
            password: password.to_string(),
        }
    }

    pub fn rpc_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }
}

/// The nodes making up the environment: the standalone node, and the one the
/// GUI runs.
pub fn get_all_nodes() -> Vec<NodeConfig> {
    vec![NodeConfig::from_env("NODE1"), NodeConfig::from_env("GUI")]
}

/// Create an RPC client for a node
pub fn create_client(config: &NodeConfig) -> Result<Client, jsonrpc::simple_http::Error> {
    Client::new(&config.rpc_url(), &config.user, &config.password)
}

/// Create an RPC client addressed at the workload's wallet on a node.
///
/// Wallet RPCs are named explicitly rather than left to the node's default,
/// because exploring the interface can load a second wallet — creating one is a
/// few clicks away — and an unqualified wallet RPC fails as soon as more than
/// one is loaded.
pub fn create_wallet_client(config: &NodeConfig) -> Result<Client, jsonrpc::simple_http::Error> {
    Client::new(
        &format!("{}/wallet/{}", config.rpc_url(), WALLET_NAME),
        &config.user,
        &config.password,
    )
}

/// Get a random u64 from Antithesis
pub fn random_u64() -> u64 {
    antithesis_sdk::random::get_random()
}

/// Get a random value in range [0, max)
pub fn random_range(max: u64) -> u64 {
    if max == 0 {
        return 0;
    }
    random_u64() % max
}

/// Pick a random node, and return the other one alongside it.
///
/// Every driver picks its nodes this way, so no direction of traffic is
/// hard-coded: the GUI both sends and receives, and mines and follows.
pub fn random_node_pair(nodes: &[NodeConfig]) -> (&NodeConfig, &NodeConfig) {
    let index = random_range(nodes.len() as u64) as usize;
    let other = (index + 1) % nodes.len();
    (&nodes[index], &nodes[other])
}

/// Generate a random amount between 0.001 and 1 BTC, rounded to satoshi precision
pub fn random_amount() -> f64 {
    round_to_satoshis(0.001 + (random_range(999) as f64) * 0.001)
}

/// Round to 8 decimal places (satoshi precision) to avoid floating point issues
pub fn round_to_satoshis(amount: f64) -> f64 {
    (amount * 100_000_000.0).round() / 100_000_000.0
}

/// Create the workload's wallet on a node, loading it if it already exists.
///
/// Every driver does this before it touches the wallet: the wallet can be
/// unloaded from the interface while the run is going, and a driver that gave
/// up at that point would leave the rest of the run without any traffic.
pub fn ensure_wallet(client: &Client, node: &str) {
    if client
        .call::<serde_json::Value>("createwallet", &[serde_json::json!(WALLET_NAME)])
        .is_ok()
    {
        println!("{}: wallet created", node);
        return;
    }

    match client.call::<serde_json::Value>("loadwallet", &[serde_json::json!(WALLET_NAME)]) {
        Ok(_) => println!("{}: wallet loaded", node),
        // Already loaded is the common case here: the wallet outlives the
        // command that created it.
        Err(e) => println!("{}: wallet already available ({})", node, e),
    }
}

/// The wallet both nodes use. The GUI shows it as the active wallet, so the
/// balance and transaction list on screen are the ones the drivers move.
pub const WALLET_NAME: &str = "default";

/// Get mempool info from a node
pub fn get_mempool_info(client: &Client) -> Result<MempoolInfo, jsonrpc::Error> {
    client.call("getmempoolinfo", &[])
}

/// Get wallet info from a node
pub fn get_wallet_info(client: &Client) -> Result<WalletInfo, jsonrpc::Error> {
    client.call("getwalletinfo", &[])
}

/// Assert sometimes conditions for mempool metrics
pub fn assert_mempool_metrics(client: &Client, context: &str) {
    if let Ok(mempool) = get_mempool_info(client) {
        antithesis_sdk::assert_sometimes_greater_than!(
            mempool.size,
            0,
            "Mempool has transactions",
            &serde_json::json!({ "context": context })
        );

        antithesis_sdk::assert_sometimes_greater_than!(
            mempool.size,
            100,
            "Mempool has more than 100 transactions",
            &serde_json::json!({ "context": context })
        );
    }
}

/// Assert sometimes conditions for wallet metrics
pub fn assert_wallet_metrics(client: &Client, context: &str) {
    if let Ok(wallet) = get_wallet_info(client) {
        println!("Wallet info for context '{}': {:?}", context, wallet);

        antithesis_sdk::assert_sometimes_greater_than!(
            wallet.txcount,
            0,
            "Wallet has transactions",
            &serde_json::json!({ "context": context })
        );

        antithesis_sdk::assert_sometimes_greater_than!(
            wallet.txcount,
            100,
            "Wallet has more than 100 transactions",
            &serde_json::json!({ "context": context })
        );
    } else {
        println!("Failed to get wallet info for context '{}'", context);
    }
}
