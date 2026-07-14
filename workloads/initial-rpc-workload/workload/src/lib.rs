use std::env;
use std::path::PathBuf;

use jsonrpc::simple_http::SimpleHttpTransport;
use jsonrpc::Client as JsonRpcClient;
use serde::Deserialize;
use serde_json::value::RawValue;

pub mod ipc;

/// Response from getchaintips RPC
#[derive(Debug, Deserialize, Clone)]
pub struct ChainTip {
    pub height: u64,
    pub hash: String,
    pub branchlen: u64,
    pub status: String, // "active", "valid-fork", "valid-headers", "headers-only", "invalid"
}

/// Response from getmempoolinfo RPC
#[derive(Debug, Deserialize, Clone)]
pub struct MempoolInfo {
    pub size: u64,          // Number of transactions
    pub bytes: u64,         // Size in bytes
    pub usage: u64,         // Total memory usage
    pub maxmempool: u64,    // Maximum mempool size
    pub mempoolminfee: f64, // Minimum fee rate
}

/// Reorg detection result
#[derive(Debug, Clone)]
pub struct ReorgInfo {
    pub fork_count: usize,   // Number of valid forks (excluding active)
    pub max_fork_depth: u64, // Maximum branchlen among forks
}

/// Response from getwalletinfo RPC
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct WalletInfo {
    pub walletname: String,
    pub txcount: u64,
    pub keypoolsize: u64,
    pub keypoolsize_hd_internal: Option<u64>,
    pub paytxfee: f64,
    pub private_keys_enabled: bool,
    pub avoid_reuse: bool,
    pub descriptors: bool,
    pub external_signer: bool,
    pub blank: bool,
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

/// Get all node configurations from environment
pub fn get_all_nodes() -> Vec<NodeConfig> {
    vec![
        NodeConfig::from_env("NODE1"),
        NodeConfig::from_env("NODE2"),
        NodeConfig::from_env("NODE3"),
    ]
}

/// Create an RPC client for a node
pub fn create_client(config: &NodeConfig) -> Result<Client, jsonrpc::simple_http::Error> {
    Client::new(&config.rpc_url(), &config.user, &config.password)
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

/// Pick a random node from the list
pub fn random_node(nodes: &[NodeConfig]) -> &NodeConfig {
    let idx = random_range(nodes.len() as u64) as usize;
    &nodes[idx]
}

/// Generate a random amount between 0.001 and 1 BTC, rounded to satoshi precision
pub fn random_amount() -> f64 {
    round_to_satoshis(0.001 + (random_range(999) as f64) * 0.001)
}

/// Round to 8 decimal places (satoshi precision) to avoid floating point issues
pub fn round_to_satoshis(amount: f64) -> f64 {
    (amount * 100_000_000.0).round() / 100_000_000.0
}

/// Get chain tips from a node
pub fn get_chain_tips(client: &Client) -> Result<Vec<ChainTip>, jsonrpc::Error> {
    client.call("getchaintips", &[])
}

/// Analyze chain tips to detect reorg information
pub fn analyze_reorgs(tips: &[ChainTip]) -> ReorgInfo {
    let forks: Vec<&ChainTip> = tips
        .iter()
        .filter(|tip| tip.status == "valid-fork" || tip.status == "valid-headers")
        .collect();

    let fork_count = forks.len();
    let max_fork_depth = forks.iter().map(|t| t.branchlen).max().unwrap_or(0);

    ReorgInfo {
        fork_count,
        max_fork_depth,
    }
}

/// Get mempool info from a node
pub fn get_mempool_info(client: &Client) -> Result<MempoolInfo, jsonrpc::Error> {
    client.call("getmempoolinfo", &[])
}

/// Get wallet info from a node
pub fn get_wallet_info(client: &Client) -> Result<WalletInfo, jsonrpc::Error> {
    client.call("getwalletinfo", &[])
}

/// Assert sometimes conditions for reorg metrics
pub fn assert_reorg_metrics(client: &Client, context: &str) {
    if let Ok(tips) = get_chain_tips(client) {
        let reorg_info = analyze_reorgs(&tips);

        // Fork depth ladder: shows how deep reorg coverage actually gets.
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.fork_count,
            0,
            "Reorg detected: at least one fork exists",
            &serde_json::json!({ "context": context })
        );
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.max_fork_depth,
            1,
            "Fork depth greater than 1",
            &serde_json::json!({ "context": context })
        );
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.max_fork_depth,
            6,
            "Fork depth greater than 6",
            &serde_json::json!({ "context": context })
        );
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.max_fork_depth,
            16,
            "Fork depth greater than 16",
            &serde_json::json!({ "context": context })
        );

        // Fork count ladder: stale-tip entries accumulate over the run, so
        // higher counts indicate sustained block-race / reorg activity.
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.fork_count,
            1,
            "Multiple forks exist simultaneously",
            &serde_json::json!({ "context": context })
        );
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.fork_count,
            16,
            "More than 16 forks exist simultaneously",
            &serde_json::json!({ "context": context })
        );
    }
}

/// Assert sometimes conditions for mempool metrics
pub fn assert_mempool_metrics(client: &Client, context: &str) {
    if let Ok(mempool) = get_mempool_info(client) {
        // Sometimes mempool has transactions (non-empty)
        antithesis_sdk::assert_sometimes_greater_than!(
            mempool.size,
            0,
            "Mempool has transactions",
            &serde_json::json!({ "context": context })
        );

        // Sometimes mempool has significant transaction count
        antithesis_sdk::assert_sometimes_greater_than!(
            mempool.size,
            1000,
            "Mempool has more than 1000 transactions",
            &serde_json::json!({ "context": context })
        );

        // Sometimes mempool memory usage approaches -maxmempool (eviction
        // territory). Note: `bytes` (serialized size) can never exceed
        // `maxmempool`, which caps `usage` (memory usage, always >= bytes),
        // so usage is the right metric here.
        antithesis_sdk::assert_sometimes_greater_than!(
            mempool.usage,
            mempool.maxmempool * 8 / 10,
            "Mempool usage exceeds 80% of -maxmempool",
            &serde_json::json!({ "context": context })
        );
    }
}

/// IPC node configuration from environment variables
pub struct IpcNodeConfig {
    pub socket_path: PathBuf,
}

impl IpcNodeConfig {
    pub fn from_env(node_name: &str) -> Self {
        let env_var = format!("{}_IPC_SOCKET", node_name.to_uppercase());
        let path = env::var(&env_var)
            .unwrap_or_else(|_| panic!("Missing environment variable: {}", env_var));
        Self {
            socket_path: PathBuf::from(path),
        }
    }
}

/// Get all IPC node configurations from environment
pub fn get_all_ipc_nodes() -> Vec<IpcNodeConfig> {
    vec![
        IpcNodeConfig::from_env("NODE1"),
        IpcNodeConfig::from_env("NODE2"),
        IpcNodeConfig::from_env("NODE3"),
    ]
}

/// Pick a random IPC node from the list
pub fn random_ipc_node(nodes: &[IpcNodeConfig]) -> &IpcNodeConfig {
    let idx = random_range(nodes.len() as u64) as usize;
    &nodes[idx]
}

/// Assert sometimes conditions for wallet metrics
pub fn assert_wallet_metrics(client: &Client, context: &str) {
    if let Ok(wallet) = get_wallet_info(client) {
        println!("Wallet info for context '{}': {:?}", context, wallet);

        // Sometimes wallet has transactions
        antithesis_sdk::assert_sometimes_greater_than!(
            wallet.txcount,
            0,
            "Wallet has transactions",
            &serde_json::json!({ "context": context })
        );

        // Sometimes wallet has many transactions (>100)
        antithesis_sdk::assert_sometimes_greater_than!(
            wallet.txcount,
            100,
            "Wallet has more than 100 transactions",
            &serde_json::json!({ "context": context })
        );

        // Sometimes wallet has many transactions (>1000)
        antithesis_sdk::assert_sometimes_greater_than!(
            wallet.txcount,
            1000,
            "Wallet has more than 1000 transactions",
            &serde_json::json!({ "context": context })
        );
    } else {
        println!("Failed to get wallet info for context '{}'", context);
    }
}
