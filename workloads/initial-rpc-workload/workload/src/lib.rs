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

/// Subset of the getblockchaininfo RPC response
#[derive(Debug, Deserialize, Clone)]
pub struct BlockchainInfo {
    pub blocks: u64,
    pub bestblockhash: String,
    /// Total work in the active chain, as a zero padded 64 digit hex string, so
    /// lexicographic ordering matches numeric ordering.
    pub chainwork: String,
    /// Lowest-height complete block stored, i.e. all previous blocks have been
    /// pruned (only present if pruning is enabled).
    pub pruneheight: Option<u64>,
}

/// Subset of the getblockheader RPC response
#[derive(Debug, Deserialize, Clone)]
pub struct BlockHeader {
    pub hash: String,
    pub height: u64,
    /// -1 if the block is not on the node's active chain.
    pub confirmations: i64,
    pub previousblockhash: Option<String>,
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

/// Get blockchain info from a node
pub fn get_blockchain_info(client: &Client) -> Result<BlockchainInfo, jsonrpc::Error> {
    client.call("getblockchaininfo", &[])
}

/// Enable or disable a node's p2p networking
pub fn set_network_active(client: &Client, active: bool) -> Result<bool, jsonrpc::Error> {
    client.call("setnetworkactive", &[active.into()])
}

/// Get a block header from a node
pub fn get_block_header(client: &Client, hash: &str) -> Result<BlockHeader, jsonrpc::Error> {
    client.call("getblockheader", &[hash.into(), true.into()])
}

/// Height of the last block the node's active chain has in common with the
/// chain ending in `target_tip`.
///
/// The node's headers for the target chain are walked back until one of them is
/// found on the active chain. Returns `None` if the node doesn't know the target
/// chain's headers or the walk exceeds `max_depth` steps, i.e. the fork point is
/// unknown.
pub fn find_fork_height(client: &Client, target_tip: &str, max_depth: u64) -> Option<u64> {
    let mut hash = target_tip.to_string();
    for _ in 0..max_depth {
        let header = get_block_header(client, &hash).ok()?;
        if header.confirmations >= 0 {
            return Some(header.height);
        }
        hash = header.previousblockhash?;
    }
    None
}

/// Number of recent blocks a pruned node keeps available for its peers, i.e.
/// `NODE_NETWORK_LIMITED_MIN_BLOCKS`.
pub const NETWORK_LIMITED_MIN_BLOCKS: u64 = 288;

/// Slack Core allows around the limited window to avoid racing the tip.
const NETWORK_LIMITED_RACE_BUFFER: u64 = 2;

/// Whether a node can hand the block at `height` on its active chain to a peer.
///
/// An unpruned node serves its whole chain. A pruned node has no data at all
/// below `pruneheight`, and because it advertises `NODE_NETWORK_LIMITED` rather
/// than `NODE_NETWORK` it only serves roughly the last
/// `NETWORK_LIMITED_MIN_BLOCKS` blocks of what it does have. The window is sized
/// generously here so that a node is only ever excused from converging when the
/// block is certainly out of reach.
pub fn can_serve_block(info: &BlockchainInfo, height: u64) -> bool {
    let Some(pruneheight) = info.pruneheight else {
        return true;
    };
    height >= pruneheight
        && info.blocks.saturating_sub(height)
            <= NETWORK_LIMITED_MIN_BLOCKS + NETWORK_LIMITED_RACE_BUFFER
}

/// Whether a node can never reorg away from its own chain above `fork_height`
/// because it has already pruned a block it would have to disconnect.
///
/// Reorging means disconnecting every block above the fork point on the node's
/// own chain, which needs those blocks' undo data on disk. Below `pruneheight`
/// that data is gone, so the reorg can never complete and the node is
/// permanently stuck on its current chain.
pub fn disconnect_blocked_by_pruning(info: &BlockchainInfo, fork_height: u64) -> bool {
    // Blocks fork_height + 1 ..= info.blocks have to be disconnected.
    info.pruneheight
        .is_some_and(|pruneheight| fork_height + 1 < pruneheight)
}

/// Whether the blocks above `fork_height` on the most-work chain can't be
/// obtained from anyone, so a node on a different chain can never connect it.
///
/// `best_chain` are the nodes already on the most-work chain, which are the only
/// ones holding its block data: the branch above the fork point exists nowhere
/// else. If none of them will serve the first block above the fork point, a
/// lagging node is stuck on headers forever, no matter how long it waits. This
/// is what pruning a fresh fork away does — the pruned node keeps mining on a
/// chain it can no longer let anybody else follow.
pub fn download_blocked_by_pruning(fork_height: u64, best_chain: &[&BlockchainInfo]) -> bool {
    // The serve window of each node is contiguous and ends at the shared tip, so
    // the lowest block needed is the one that decides this.
    !best_chain
        .iter()
        .any(|info| can_serve_block(info, fork_height + 1))
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

/// Satoshis per BTC, i.e. `COIN`.
pub const COIN: i64 = 100_000_000;

/// `MAX_MONEY` from consensus/amount.h, in satoshis.
pub const MAX_MONEY_SATS: i64 = 21_000_000 * COIN;

/// `consensus.nSubsidyHalvingInterval` on regtest.
const SUBSIDY_HALVING_INTERVAL: u64 = 150;

/// Block subsidy in satoshis at `height`, mirroring `GetBlockSubsidy()`.
fn block_subsidy_sats(height: u64) -> i64 {
    let halvings = height / SUBSIDY_HALVING_INTERVAL;
    if halvings >= 64 {
        return 0;
    }
    (50 * COIN) >> halvings
}

/// Total subsidy in satoshis paid into the UTXO set by a chain of `height` blocks.
///
/// Starts at height 1: the genesis coinbase never enters the UTXO set. No set of wallets
/// on one chain can hold more than this, since fees only redistribute existing coins.
pub fn total_subsidy_issued_sats(height: u64) -> i64 {
    let mut sats: i64 = 0;
    let mut h = 1u64;
    while h <= height {
        let subsidy = block_subsidy_sats(h);
        if subsidy == 0 {
            break;
        }
        // Last height in the halving epoch containing `h`.
        let epoch_end = (h / SUBSIDY_HALVING_INTERVAL + 1) * SUBSIDY_HALVING_INTERVAL - 1;
        let last = epoch_end.min(height);
        sats = sats.saturating_add(subsidy.saturating_mul((last - h + 1) as i64));
        h = last + 1;
    }
    sats
}

/// Result of validating a JSON value that Core documents as a money amount. Carried as
/// whole satoshis so that nothing downstream sums or compares amounts as floats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Money {
    /// A whole number of satoshis within `[-MAX_MONEY, MAX_MONEY]`.
    Valid(i64),
    /// Outside that range, or not finite. Reported as the raw BTC value.
    OutOfRange(f64),
    /// In range but not a whole satoshi, which is itself a defect.
    SubSatoshi(f64),
    /// Not a JSON number at all.
    NotANumber,
}

/// Largest distance, in satoshis, tolerated between a reported amount and the whole satoshi
/// it must represent. `MAX_MONEY` is 2.1e15 satoshis, inside the 2^53 range where doubles
/// represent every integer, so `btc * COIN` rounds back exactly and this only absorbs the
/// decimal literal's representation error.
const SATOSHI_EPSILON: f64 = 0.01;

/// Validate a field Core documents as a money amount and convert it to satoshis.
///
/// The bound is sign-agnostic — a `send` amount and a fee are legitimately negative — so
/// callers check the sign themselves where Core documents one.
pub fn check_money(value: &serde_json::Value) -> Money {
    let Some(btc) = value.as_f64() else {
        return Money::NotANumber;
    };
    let sats = btc * COIN as f64;
    if !sats.is_finite() || sats.abs() > MAX_MONEY_SATS as f64 {
        return Money::OutOfRange(btc);
    }
    if (sats - sats.round()).abs() > SATOSHI_EPSILON {
        return Money::SubSatoshi(btc);
    }
    Money::Valid(sats.round() as i64)
}

/// Format satoshis as BTC for logging. Never used for comparisons.
pub fn sats_to_btc_string(sats: i128) -> String {
    let sign = if sats < 0 { "-" } else { "" };
    let abs = sats.unsigned_abs();
    let coin = COIN as u128;
    format!("{}{}.{:08}", sign, abs / coin, abs % coin)
}

/// The balance buckets of one wallet, i.e. `getbalances.mine`. `used` only appears on
/// `avoid_reuse` wallets, where those coins are excluded from `trusted`, so it is not
/// double counted.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct BalanceBuckets {
    pub trusted: serde_json::Value,
    pub untrusted_pending: serde_json::Value,
    pub immature: serde_json::Value,
    pub used: serde_json::Value,
}

impl BalanceBuckets {
    /// The buckets as (name, raw value) pairs, in report order.
    pub fn fields(&self) -> [(&'static str, &serde_json::Value); 4] {
        [
            ("trusted", &self.trusted),
            ("untrusted_pending", &self.untrusted_pending),
            ("immature", &self.immature),
            ("used", &self.used),
        ]
    }
}

/// `getbalances.lastprocessedblock`, the block a wallet has scanned up to.
#[derive(Debug, Deserialize, Clone)]
pub struct LastProcessedBlock {
    pub height: u64,
}

/// Subset of the getbalances RPC response
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Balances {
    pub mine: BalanceBuckets,
    /// Absent on older branches; callers fall back to the chain height.
    pub lastprocessedblock: Option<LastProcessedBlock>,
}

/// Get balances from a node's loaded wallet
pub fn get_balances(client: &Client) -> Result<Balances, jsonrpc::Error> {
    client.call("getbalances", &[])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsidy_matches_regtest_schedule() {
        assert_eq!(total_subsidy_issued_sats(0), 0);
        assert_eq!(total_subsidy_issued_sats(1), 50 * COIN);
        // The first halving takes effect *at* height 150.
        assert_eq!(total_subsidy_issued_sats(149), 149 * 50 * COIN);
        assert_eq!(total_subsidy_issued_sats(150), 149 * 50 * COIN + 25 * COIN);
        assert_eq!(
            total_subsidy_issued_sats(299),
            149 * 50 * COIN + 150 * 25 * COIN
        );
        assert_eq!(
            total_subsidy_issued_sats(300),
            149 * 50 * COIN + 150 * 25 * COIN + 12 * COIN + 50_000_000
        );
    }

    #[test]
    fn money_is_converted_to_exact_satoshis() {
        for (btc, sats) in [
            (serde_json::json!(1.5), 150_000_000),
            (serde_json::json!(-1.5), -150_000_000),
            (serde_json::json!(0), 0),
            (serde_json::json!(0.00000001), 1),
            (serde_json::json!(-0.00000001), -1),
            (serde_json::json!(0.1), 10_000_000),
            (serde_json::json!(20999999.99999999), 2_099_999_999_999_999),
            (serde_json::json!(21000000.0), MAX_MONEY_SATS),
            (serde_json::json!(-21000000.0), -MAX_MONEY_SATS),
        ] {
            assert_eq!(check_money(&btc), Money::Valid(sats), "for {}", btc);
        }
    }

    #[test]
    fn money_range_is_enforced() {
        // One satoshi over MAX_MONEY, i.e. what an overflowed sum walks into.
        assert_eq!(
            check_money(&serde_json::json!(21000000.00000001)),
            Money::OutOfRange(21000000.00000001)
        );
        assert_eq!(
            check_money(&serde_json::json!(92233720368.54776)),
            Money::OutOfRange(92233720368.54776)
        );
        assert!(matches!(
            check_money(&serde_json::json!(-92233720368.54776)),
            Money::OutOfRange(_)
        ));
        assert_eq!(
            check_money(&serde_json::json!(0.000000005)),
            Money::SubSatoshi(0.000000005)
        );
        assert_eq!(check_money(&serde_json::json!("1.5")), Money::NotANumber);
        assert_eq!(check_money(&serde_json::Value::Null), Money::NotANumber);
    }

    #[test]
    fn accumulating_satoshis_is_exact() {
        fn sats(value: serde_json::Value) -> i128 {
            match check_money(&value) {
                Money::Valid(sats) => sats as i128,
                other => panic!("unexpected {:?}", other),
            }
        }

        // Why integers: as f64 these two amounts exceed the total they add up to, and
        // the cluster supply bound is tight enough for that drift to report a violation
        // that never happened.
        let as_float = |value: serde_json::Value| value.as_f64().unwrap();
        assert!(
            as_float(serde_json::json!(0.1)) + as_float(serde_json::json!(0.2))
                > as_float(serde_json::json!(0.3))
        );
        assert_eq!(
            sats(serde_json::json!(0.1)) + sats(serde_json::json!(0.2)),
            sats(serde_json::json!(0.3))
        );

        let total: i128 = (0..30).map(|_| sats(serde_json::json!(0.1))).sum();
        assert_eq!(total, 3 * COIN as i128);
        assert_eq!(sats_to_btc_string(total), "3.00000000");
        assert_eq!(sats_to_btc_string(-1), "-0.00000001");
        assert_eq!(
            sats_to_btc_string(MAX_MONEY_SATS as i128),
            "21000000.00000000"
        );
    }
}
