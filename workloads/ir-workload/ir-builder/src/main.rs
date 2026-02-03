use antithesis_sdk::random::AntithesisRng;
use fuzzamoto::connections::{Connection, ConnectionType, HandshakeOpts, V1Transport};
use fuzzamoto_ir::{
    FullProgramContext, Generator, Mutator, MutatorError, PerTestcaseMetadata, Program,
    ProgramBuilder, RecentBlock,
    compiler::{CompiledAction, Compiler},
    generators::{
        // Address
        AddrRelayGenerator,
        AddrRelayV2Generator,
        // Block
        BlockGenerator,
        TipBlockGenerator,
        BloomFilterAddGenerator,
        BloomFilterClearGenerator,
        // Bloom
        BloomFilterLoadGenerator,
        // Compact
        CompactBlockGenerator,
        CompactFilterQueryGenerator,
        // Other
        GetAddrGenerator,
        // Existing
        GetDataGenerator,
        HeaderGenerator,
        // Inventory
        InventoryGenerator,
        LargeTxGenerator,
        LongChainGenerator,
        // Transaction
        OneParentOneChildGenerator,
        SendBlockGenerator,
        SendMessageGenerator,
        SingleTxGenerator,
        TxoGenerator,
    },
    mutators::{InputMutator, OperationMutator},
};
use jsonrpc::Client as JsonRpcClient;
use jsonrpc::simple_http::SimpleHttpTransport;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Response from getchaintips RPC
#[derive(Debug, Deserialize, Clone)]
struct ChainTip {
    #[allow(dead_code)]
    height: u64,
    #[allow(dead_code)]
    hash: String,
    branchlen: u64,
    status: String, // "active", "valid-fork", "valid-headers", "headers-only", "invalid"
}

/// Response from getmempoolinfo RPC
#[derive(Debug, Deserialize, Clone)]
struct MempoolInfo {
    size: u64,  // Number of transactions
    bytes: u64, // Size in bytes
    #[allow(dead_code)]
    usage: u64, // Total memory usage
    maxmempool: u64, // Maximum mempool size
    #[allow(dead_code)]
    mempoolminfee: f64, // Minimum fee rate
}

/// Response from getblockheader RPC
#[derive(Debug, Deserialize, Clone)]
struct BlockHeaderInfo {
    height: u64,
    #[allow(dead_code)]
    hash: String,
}

/// Reorg detection result
#[derive(Debug, Clone)]
struct ReorgInfo {
    fork_count: usize,   // Number of valid forks (excluding active)
    max_fork_depth: u64, // Maximum branchlen among forks
}

/// Simple Bitcoin RPC client
struct RpcClient {
    inner: JsonRpcClient,
}

impl RpcClient {
    fn new(url: &str, user: &str, password: &str) -> Result<Self, String> {
        let transport = SimpleHttpTransport::builder()
            .url(url)
            .map_err(|e| format!("Invalid RPC URL '{}': {}", url, e))?
            .auth(user, Some(password))
            .build();
        Ok(Self {
            inner: JsonRpcClient::with_transport(transport),
        })
    }

    fn from_url(url: &str) -> Result<Self, String> {
        // Parse URL like http://user:password@host:port
        let url = url.trim_start_matches("http://");
        let (auth, host_port) = url.split_once('@').ok_or("URL must contain @")?;
        let (user, password) = auth.split_once(':').ok_or("Auth must be user:password")?;
        let rpc_url = format!("http://{}", host_port);
        Self::new(&rpc_url, user, password)
    }

    fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<T, String> {
        let params = if args.is_empty() {
            None
        } else {
            let serialized = serde_json::to_string(args).expect("failed to serialize args");
            Some(RawValue::from_string(serialized).expect("failed to create RawValue"))
        };
        let request = self.inner.build_request(method, params.as_deref());
        let response = self
            .inner
            .send_request(request)
            .map_err(|e| format!("RPC error: {:?}", e))?;
        response
            .result()
            .map_err(|e| format!("RPC result error: {:?}", e))
    }

    fn get_chain_tips(&self) -> Result<Vec<ChainTip>, String> {
        self.call("getchaintips", &[])
    }

    fn get_mempool_info(&self) -> Result<MempoolInfo, String> {
        self.call("getmempoolinfo", &[])
    }

    fn get_block_header(&self, block_hash: &str) -> Result<BlockHeaderInfo, String> {
        self.call("getblockheader", &[block_hash.into(), true.into()])
    }

    fn get_best_block_hash(&self) -> Result<String, String> {
        self.call("getbestblockhash", &[])
    }
}

/// Analyze chain tips to detect reorg information
fn analyze_reorgs(tips: &[ChainTip]) -> ReorgInfo {
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

/// Get active chain height from chain tips
fn get_active_chain_height(tips: &[ChainTip]) -> Option<u64> {
    tips.iter()
        .find(|tip| tip.status == "active")
        .map(|tip| tip.height)
}

/// Assert sometimes conditions for chain height (guides fuzzer towards higher heights)
fn assert_chain_height_metrics(client: &RpcClient, context: &str) {
    if let Ok(tips) = client.get_chain_tips() {
        if let Some(height) = get_active_chain_height(&tips) {
            // Guide towards chains with progressively higher heights
            antithesis_sdk::assert_sometimes_greater_than!(
                height,
                10,
                "Chain height greater than 10",
                &serde_json::json!({ "context": context, "height": height })
            );

            antithesis_sdk::assert_sometimes_greater_than!(
                height,
                100,
                "Chain height greater than 100",
                &serde_json::json!({ "context": context, "height": height })
            );

            antithesis_sdk::assert_sometimes_greater_than!(
                height,
                500,
                "Chain height greater than 500",
                &serde_json::json!({ "context": context, "height": height })
            );

            antithesis_sdk::assert_sometimes_greater_than!(
                height,
                1000,
                "Chain height greater than 1000",
                &serde_json::json!({ "context": context, "height": height })
            );

            antithesis_sdk::assert_sometimes_greater_than!(
                height,
                5000,
                "Chain height greater than 5000",
                &serde_json::json!({ "context": context, "height": height })
            );
        }
    }
}

/// Assert sometimes conditions for reorg metrics
fn assert_reorg_metrics(client: &RpcClient, context: &str) {
    if let Ok(tips) = client.get_chain_tips() {
        let reorg_info = analyze_reorgs(&tips);

        // Sometimes we see at least one fork (indicates reorg activity)
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.fork_count,
            0,
            "Reorg detected: at least one fork exists",
            &serde_json::json!({ "context": context })
        );

        // Sometimes we see deep reorgs (depth > 16)
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.max_fork_depth,
            16,
            "Deep reorg detected: fork depth greater than 16",
            &serde_json::json!({ "context": context })
        );

        // Sometimes we see multiple forks simultaneously
        antithesis_sdk::assert_sometimes_greater_than!(
            reorg_info.fork_count,
            16,
            "Multiple forks detected simultaneously",
            &serde_json::json!({ "context": context })
        );
    }
}

/// Assert sometimes conditions for mempool metrics
fn assert_mempool_metrics(client: &RpcClient, context: &str) {
    if let Ok(mempool) = client.get_mempool_info() {
        // Sometimes mempool has transactions (non-empty)
        antithesis_sdk::assert_sometimes_greater_than!(
            mempool.size,
            0,
            "Mempool has transactions",
            &serde_json::json!({ "context": context })
        );

        // Sometimes mempool has significant transaction count (>1000 txs)
        antithesis_sdk::assert_sometimes_greater_than!(
            mempool.size,
            1000,
            "Mempool has more than 1000 transactions",
            &serde_json::json!({ "context": context })
        );

        // Sometimes mempool exceeds -maxmempool in size
        antithesis_sdk::assert_sometimes_greater_than!(
            mempool.bytes,
            mempool.maxmempool,
            "Mempool exceeds -maxmempool in size",
            &serde_json::json!({ "context": context })
        );
    }
}

#[derive(Debug, Deserialize)]
struct Request {
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct Response {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl Response {
    fn success(result: serde_json::Value) -> Self {
        Self {
            success: true,
            result: Some(result),
            error: None,
        }
    }

    fn error(msg: &str) -> Self {
        Self {
            success: false,
            result: None,
            error: Some(msg.to_string()),
        }
    }
}

/// A simple byte mutator for OperationMutator
struct SimpleByteMutator<R: RngCore> {
    rng: R,
}

impl<R: RngCore> SimpleByteMutator<R> {
    fn new(rng: R) -> Self {
        Self { rng }
    }
}

impl<R: RngCore> fuzzamoto_ir::mutators::OperationByteMutator for SimpleByteMutator<R> {
    fn mutate_bytes(&mut self, bytes: &mut Vec<u8>) {
        if bytes.is_empty() {
            bytes.push(self.rng.r#gen());
            return;
        }
        // Sometimes change the size and fill, sometimes just fill
        if self.rng.gen_bool(0.5) {
            let new_size = self.rng.gen_range(1..=bytes.len() + 32);
            bytes.resize(new_size, 0);
        }
        self.rng.fill_bytes(&mut bytes[0..]);
    }
}

/// Metadata for a P2P connection to track how to reconnect
#[derive(Clone)]
struct ConnectionMeta {
    /// Whether this is an inbound (we connect to node) or outbound (node connects to us) connection
    is_inbound: bool,
    /// Which node this connection is to (1-indexed)
    node_num: usize,
    /// Timestamp for handshake
    timestamp: u64,
}

/// State maintained by the ir-builder
struct IrBuilderState {
    /// Program builder (maintains program + variable tracking)
    builder: Option<ProgramBuilder>,
    /// Random number generator using Antithesis RNG
    rng: AntithesisRng,
    /// Per-testcase metadata
    metadata: PerTestcaseMetadata,
    /// Full program context (available txos, headers, etc.)
    full_context: Option<FullProgramContext>,
    /// P2P connections to nodes (indexed by connection id)
    connections: Vec<Connection<V1Transport>>,
    /// Metadata for each connection (same index as connections)
    connection_metas: Vec<ConnectionMeta>,
    /// Persistent compiler instance for streaming compilation
    compiler: Option<Compiler>,
    /// Current tip height for TipBlockGenerator tracking
    current_tip_height: u64,
}

impl IrBuilderState {
    fn new() -> Self {
        Self {
            builder: None,
            rng: AntithesisRng,
            metadata: PerTestcaseMetadata::new(),
            full_context: None,
            connections: Vec::new(),
            connection_metas: Vec::new(),
            compiler: None,
            current_tip_height: 0,
        }
    }

    /// Create an inbound connection (we connect to node's P2P port)
    fn create_inbound_connection(
        node_num: usize,
        timestamp: u64,
    ) -> Result<Connection<V1Transport>, String> {
        let env_var = format!("NODE{}_P2P_ADDR", node_num);
        let p2p_addr = std::env::var(&env_var)
            .map_err(|_| format!("Missing environment variable: {}", env_var))?;

        let socket = TcpStream::connect(&p2p_addr)
            .map_err(|e| format!("Failed to connect to {}: {}", p2p_addr, e))?;

        socket
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("Failed to set read timeout: {}", e))?;
        socket
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("Failed to set write timeout: {}", e))?;
        socket
            .set_nodelay(true)
            .map_err(|e| format!("Failed to set nodelay: {}", e))?;

        let transport = V1Transport { socket };
        let mut connection = Connection::new(ConnectionType::Inbound, transport);

        let handshake_opts = HandshakeOpts {
            time: timestamp as i64,
            relay: true,
            starting_height: 0,
            wtxidrelay: true,
            addrv2: true,
            erlay: false,
        };
        connection
            .version_handshake(handshake_opts)
            .map_err(|e| format!("Handshake failed: {}", e))?;

        Ok(connection)
    }

    /// Create an outbound connection (node connects to us via addconnection RPC)
    fn create_outbound_connection(
        node_num: usize,
        timestamp: u64,
    ) -> Result<Connection<V1Transport>, String> {
        let env_var = format!("NODE{}_RPC_URL", node_num);
        let rpc_url = std::env::var(&env_var)
            .map_err(|_| format!("Missing environment variable: {}", env_var))?;

        let rpc_client = RpcClient::from_url(&rpc_url)?;

        // Get our hostname
        let our_hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "ir-builder".to_string());

        // Create listener on a random port
        let listener = TcpListener::bind("0.0.0.0:0")
            .map_err(|e| format!("Failed to create TCP listener: {}", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to get listener address: {}", e))?
            .port();

        // Tell Bitcoin Core to connect to our listener
        let connect_addr = format!("{}:{}", our_hostname, port);
        rpc_client.call::<serde_json::Value>(
            "addconnection",
            &[
                connect_addr.clone().into(),
                "outbound-full-relay".into(),
                false.into(),
            ],
        )?;

        // Wait for Bitcoin Core to connect
        let (socket, _addr) = listener
            .accept()
            .map_err(|e| format!("Failed to accept connection: {}", e))?;

        socket
            .set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("Failed to set read timeout: {}", e))?;
        socket
            .set_write_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("Failed to set write timeout: {}", e))?;
        socket
            .set_nodelay(true)
            .map_err(|e| format!("Failed to set nodelay: {}", e))?;

        let transport = V1Transport { socket };
        let mut connection = Connection::new(ConnectionType::Outbound, transport);

        let handshake_opts = HandshakeOpts {
            time: timestamp as i64,
            relay: true,
            starting_height: 0,
            wtxidrelay: true,
            addrv2: true,
            erlay: false,
        };
        connection
            .version_handshake(handshake_opts)
            .map_err(|e| format!("Handshake failed: {}", e))?;

        Ok(connection)
    }

    /// Reconnect a connection at the given index
    fn reconnect(&mut self, conn_idx: usize) -> Result<(), String> {
        if conn_idx >= self.connection_metas.len() {
            return Err(format!("Invalid connection index: {}", conn_idx));
        }

        let meta = self.connection_metas[conn_idx].clone();
        let new_conn = if meta.is_inbound {
            Self::create_inbound_connection(meta.node_num, meta.timestamp)?
        } else {
            Self::create_outbound_connection(meta.node_num, meta.timestamp)?
        };

        self.connections[conn_idx] = new_conn;
        println!(
            "ir-builder: reconnected {} connection {} to node{}",
            if meta.is_inbound { "inbound" } else { "outbound" },
            conn_idx,
            meta.node_num
        );

        Ok(())
    }

    fn init(&mut self, params: serde_json::Value) -> Result<serde_json::Value, String> {
        // Parse FullProgramContext from params
        let full_context: FullProgramContext = serde_json::from_value(params)
            .map_err(|e| format!("Failed to parse FullProgramContext: {}", e))?;

        let context = full_context.context.clone();

        self.connections.clear();
        self.connection_metas.clear();

        // Get our hostname (ir-builder in docker environment)
        let our_hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "ir-builder".to_string());

        // Set up 4 outbound connections (node connects to us)
        // Uses ConnectionType::Outbound - node sends version first
        let num_outbound = 4;
        for i in 0..num_outbound {
            let node_num = (i % context.num_nodes) + 1;
            let env_var = format!("NODE{}_RPC_URL", node_num);
            let rpc_url = std::env::var(&env_var)
                .map_err(|_| format!("Missing environment variable: {}", env_var))?;

            println!(
                "ir-builder: setting up outbound connection {} to node{}",
                i, node_num
            );

            let rpc_client = RpcClient::from_url(&rpc_url)?;

            // Create listener on a random port
            let listener = TcpListener::bind("0.0.0.0:0")
                .map_err(|e| format!("Failed to create TCP listener: {}", e))?;
            let port = listener
                .local_addr()
                .map_err(|e| format!("Failed to get listener address: {}", e))?
                .port();

            println!(
                "ir-builder: listening on port {} for outbound connection {}",
                port, i
            );

            // Tell Bitcoin Core to connect to our listener
            let connect_addr = format!("{}:{}", our_hostname, port);
            println!(
                "ir-builder: calling addconnection RPC to connect to {}",
                connect_addr
            );
            rpc_client.call::<serde_json::Value>(
                "addconnection",
                &[
                    connect_addr.clone().into(),
                    "outbound-full-relay".into(),
                    false.into(), // no v2
                ],
            )?;

            // Wait for Bitcoin Core to connect
            println!("ir-builder: waiting for node{} to connect...", node_num);
            let (socket, addr) = listener
                .accept()
                .map_err(|e| format!("Failed to accept connection: {}", e))?;

            println!(
                "ir-builder: accepted outbound connection {} from {}",
                i, addr
            );

            socket
                .set_read_timeout(Some(Duration::from_secs(30)))
                .map_err(|e| format!("Failed to set read timeout: {}", e))?;
            socket
                .set_write_timeout(Some(Duration::from_secs(30)))
                .map_err(|e| format!("Failed to set write timeout: {}", e))?;
            socket
                .set_nodelay(true)
                .map_err(|e| format!("Failed to set nodelay: {}", e))?;

            let transport = V1Transport { socket };
            // Use Outbound type - the node initiated the connection to us,
            // so it will send version first (we wait for it)
            let mut connection = Connection::new(ConnectionType::Outbound, transport);

            println!(
                "ir-builder: starting handshake for outbound connection {}",
                i
            );

            let handshake_opts = HandshakeOpts {
                time: context.timestamp as i64,
                relay: true,
                starting_height: 0,
                wtxidrelay: true,
                addrv2: true,
                erlay: false,
            };
            connection
                .version_handshake(handshake_opts)
                .map_err(|e| format!("Handshake failed for outbound connection {}: {}", i, e))?;

            println!("ir-builder: outbound connection {} established", i);
            self.connections.push(connection);
            self.connection_metas.push(ConnectionMeta {
                is_inbound: false,
                node_num,
                timestamp: context.timestamp,
            });
        }

        // Set up 4 inbound connections (we connect to node's P2P port)
        // Uses ConnectionType::Inbound - we send version first
        let num_inbound = 4;
        for i in 0..num_inbound {
            let node_num = (i % context.num_nodes) + 1;
            let env_var = format!("NODE{}_P2P_ADDR", node_num);
            let p2p_addr = std::env::var(&env_var)
                .map_err(|_| format!("Missing environment variable: {}", env_var))?;

            println!(
                "ir-builder: setting up inbound connection {} to node{} at {}",
                i, node_num, p2p_addr
            );

            // Connect to the node's P2P port
            let socket = TcpStream::connect(&p2p_addr)
                .map_err(|e| format!("Failed to connect to {}: {}", p2p_addr, e))?;

            println!(
                "ir-builder: TCP connected to {} for inbound connection {}",
                p2p_addr, i
            );

            socket
                .set_read_timeout(Some(Duration::from_secs(30)))
                .map_err(|e| format!("Failed to set read timeout: {}", e))?;
            socket
                .set_write_timeout(Some(Duration::from_secs(30)))
                .map_err(|e| format!("Failed to set write timeout: {}", e))?;
            socket
                .set_nodelay(true)
                .map_err(|e| format!("Failed to set nodelay: {}", e))?;

            let transport = V1Transport { socket };
            // Use Inbound type - we initiated the connection,
            // so we send version first
            let mut connection = Connection::new(ConnectionType::Inbound, transport);

            println!(
                "ir-builder: starting handshake for inbound connection {}",
                i
            );

            let handshake_opts = HandshakeOpts {
                time: context.timestamp as i64,
                relay: true,
                starting_height: 0,
                wtxidrelay: true,
                addrv2: true,
                erlay: false,
            };
            connection
                .version_handshake(handshake_opts)
                .map_err(|e| format!("Handshake failed for inbound connection {}: {}", i, e))?;

            println!("ir-builder: inbound connection {} established", i);
            self.connections.push(connection);
            self.connection_metas.push(ConnectionMeta {
                is_inbound: true,
                node_num,
                timestamp: context.timestamp,
            });
        }

        let mut builder = ProgramBuilder::new(context.clone());

        // Generate initial time variable for block building
        builder.force_append_expect_output(
            vec![],
            fuzzamoto_ir::Operation::LoadTime(context.timestamp),
        );

        let instruction_count = builder.instructions.len();
        let txo_count = full_context.txos.len();
        let header_count = full_context.headers.len();

        // Set initial tip height from snapshot headers
        self.current_tip_height = full_context
            .headers
            .iter()
            .map(|h| h.height as u64)
            .max()
            .unwrap_or(0);

        self.full_context = Some(full_context);
        self.builder = Some(builder);
        self.compiler = Some(Compiler::new());

        Ok(serde_json::json!({
            "initialized": true,
            "instruction_count": instruction_count,
            "context": {
                "num_nodes": context.num_nodes,
                "num_connections": context.num_connections,
                "timestamp": context.timestamp
            },
            "txo_count": txo_count,
            "header_count": header_count,
            "connections_established": self.connections.len()
        }))
    }

    /// Mutate the current program using the specified mutator/generator.
    ///
    /// Params should contain a "type" field with the mutator/generator name:
    /// - Mutators: "InputMutator", "OperationMutator"
    /// - Generators: "SingleTxGenerator", "GetDataGenerator"
    fn mutate(&mut self, params: serde_json::Value) -> Result<serde_json::Value, String> {
        if self.builder.is_none() {
            return Err("ir-builder not initialized".to_string());
        }

        let mutator_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'type' parameter")?;

        match mutator_type {
            // Mutators - need to finalize, mutate, and recreate builder
            "InputMutator" => {
                let mut mutator = InputMutator::new();
                let name = <InputMutator as Mutator<AntithesisRng>>::name(&mutator);
                self.apply_mutator(name, |program, rng, meta| {
                    mutator.mutate(program, rng, meta)
                })
            }

            "OperationMutator" => {
                let byte_mutator = SimpleByteMutator::new(AntithesisRng);
                let mut mutator = OperationMutator::new(byte_mutator);
                let name = <OperationMutator<_> as Mutator<AntithesisRng>>::name(&mutator);
                self.apply_mutator(name, |program, rng, meta| {
                    mutator.mutate(program, rng, meta)
                })
            }

            // Generators - use builder directly
            "SingleTxGenerator" => {
                let generator = SingleTxGenerator::default();
                self.apply_generator(&generator)
            }
            "GetDataGenerator" => {
                let generator = GetDataGenerator;
                self.apply_generator(&generator)
            }
            "TxoGenerator" => {
                let full_context = self
                    .full_context
                    .as_ref()
                    .ok_or("No full context available")?;
                let generator = TxoGenerator::new(full_context.txos.clone());
                self.apply_generator(&generator)
            }
            "HeaderGenerator" => {
                let full_context = self
                    .full_context
                    .as_ref()
                    .ok_or("No full context available")?;
                let generator = HeaderGenerator::new(full_context.headers.clone());
                self.apply_generator(&generator)
            }

            // Transaction generators
            "OneParentOneChildGenerator" => {
                let generator = OneParentOneChildGenerator::default();
                self.apply_generator(&generator)
            }
            "LongChainGenerator" => {
                let generator = LongChainGenerator::default();
                self.apply_generator(&generator)
            }
            "LargeTxGenerator" => {
                let generator = LargeTxGenerator::default();
                self.apply_generator(&generator)
            }

            // Block generators
            "BlockGenerator" => {
                let generator = BlockGenerator::default();
                self.apply_generator(&generator)
            }
            "TipBlockGenerator" => {
                let full_context = self
                    .full_context
                    .as_ref()
                    .ok_or("No full context available")?;
                let generator = TipBlockGenerator::new(full_context.headers.clone());
                // Metadata is synced from compiler + RPC in compile(), so use current metadata
                self.apply_generator(&generator)
            }
            "SendBlockGenerator" => {
                let generator = SendBlockGenerator::default();
                self.apply_generator(&generator)
            }

            // Inventory generator
            "InventoryGenerator" => {
                let generator = InventoryGenerator::default();
                self.apply_generator(&generator)
            }

            // Compact block/filter generators
            "CompactBlockGenerator" => {
                let generator = CompactBlockGenerator::default();
                self.apply_generator(&generator)
            }
            "CompactFilterQueryGenerator" => {
                let generator = CompactFilterQueryGenerator::default();
                self.apply_generator(&generator)
            }

            // Bloom filter generators
            "BloomFilterLoadGenerator" => {
                let generator = BloomFilterLoadGenerator::default();
                self.apply_generator(&generator)
            }
            "BloomFilterAddGenerator" => {
                let generator = BloomFilterAddGenerator::default();
                self.apply_generator(&generator)
            }
            "BloomFilterClearGenerator" => {
                let generator = BloomFilterClearGenerator::default();
                self.apply_generator(&generator)
            }

            // Address relay generators
            "AddrRelayGenerator" => {
                let generator = AddrRelayGenerator::new(vec![]);
                self.apply_generator(&generator)
            }
            "AddrRelayV2Generator" => {
                let generator = AddrRelayV2Generator::new(vec![]);
                self.apply_generator(&generator)
            }

            // Other generators
            "GetAddrGenerator" => {
                let generator = GetAddrGenerator::default();
                self.apply_generator(&generator)
            }
            "SendMessageGenerator" => {
                let generator = SendMessageGenerator::new(vec![
                    "ping".to_string(),
                    "mempool".to_string(),
                    "sendcmpct".to_string(),
                    "sendheaders".to_string(),
                ]); // reduced set
                self.apply_generator(&generator)
            }

            _ => Err(format!("Unknown mutator/generator type: {}", mutator_type)),
        }
    }

    fn apply_mutator<F>(
        &mut self,
        name: &'static str,
        mut mutate_fn: F,
    ) -> Result<serde_json::Value, String>
    where
        F: FnMut(
            &mut Program,
            &mut AntithesisRng,
            Option<&PerTestcaseMetadata>,
        ) -> Result<(), MutatorError>,
    {
        let builder = self.builder.as_ref().ok_or("No builder")?;

        // Get current program from builder
        let mut program = builder
            .finalize()
            .map_err(|e| format!("Failed to get program: {:?}", e))?;

        // Apply mutation
        match mutate_fn(&mut program, &mut self.rng, Some(&self.metadata)) {
            Ok(()) => {
                let instruction_count = program.instructions.len();
                // Recreate builder from mutated program
                self.builder = Some(
                    ProgramBuilder::from_program(program)
                        .map_err(|e| format!("Failed to recreate builder: {:?}", e))?,
                );
                Ok(serde_json::json!({
                    "mutated": true,
                    "type": name,
                    "instruction_count": instruction_count
                }))
            }
            Err(e) => Ok(serde_json::json!({
                "mutated": false,
                "type": name,
                "reason": format!("{:?}", e)
            })),
        }
    }

    fn apply_generator<G: Generator<AntithesisRng>>(
        &mut self,
        generator: &G,
    ) -> Result<serde_json::Value, String> {
        let builder = self.builder.as_mut().ok_or("No builder")?;
        let name = generator.name();

        match generator.generate(builder, &mut self.rng, Some(&self.metadata)) {
            Ok(()) => Ok(serde_json::json!({
                "mutated": true,
                "type": name,
                "instruction_count": builder.instructions.len()
            })),
            Err(e) => Ok(serde_json::json!({
                "mutated": false,
                "type": name,
                "reason": format!("{:?}", e)
            })),
        }
    }

    fn compile(&mut self, _params: serde_json::Value) -> Result<serde_json::Value, String> {
        let builder = self.builder.as_ref().ok_or("ir-builder not initialized")?;
        let full_context = self
            .full_context
            .as_ref()
            .ok_or("No full context available")?;
        let num_nodes = full_context.context.num_nodes;
        let compiler = self.compiler.as_mut().ok_or("Compiler not initialized")?;

        // Compile only instructions added since last compile (streaming)
        let new_actions = compiler
            .compile_from(builder)
            .map_err(|e| format!("Compilation failed: {:?}", e))?;

        let total_instructions = builder.instructions.len();

        // Execute only the NEW SendRawMessage actions
        let mut messages_sent = 0;
        for action in &new_actions {
            if let CompiledAction::SendRawMessage(conn_id, command, payload) = action {
                if *conn_id < self.connections.len() {
                    let message = (command.clone(), payload.clone());
                    match self.connections[*conn_id].send(&message) {
                        Ok(()) => {
                            messages_sent += 1;
                        }
                        Err(_e) => {}
                    }
                } else {
                }
            }
        }

        if self.rng.gen_bool(0.5) {
            let mut failed_indices = Vec::new();
            for (idx, connection) in self.connections.iter_mut().enumerate() {
                if connection.ping().is_err() {
                    failed_indices.push(idx);
                }
            }
            // Reconnect failed connections
            for idx in failed_indices {
                if let Err(e) = self.reconnect(idx) {
                    println!("ir-builder: failed to reconnect connection {}: {}", idx, e);
                }
            }
        }

        antithesis_sdk::assert_sometimes!(
            messages_sent > 0,
            "Messages sent to nodes",
            &serde_json::json!({ "messages_sent": messages_sent })
        );

        // Assert sometimes conditions for mempool, reorg, and chain height metrics on each node
        for node_num in 1..=num_nodes {
            let env_var = format!("NODE{}_RPC_URL", node_num);
            if let Ok(rpc_url) = std::env::var(&env_var) {
                if let Ok(client) = RpcClient::from_url(&rpc_url) {
                    let context_str = format!("ir-builder compile node{}", node_num);
                    assert_mempool_metrics(&client, &context_str);
                    assert_reorg_metrics(&client, &context_str);
                    assert_chain_height_metrics(&client, &context_str);
                }
            }
        }

        // NO RESET - streaming mode: instructions accumulate across compiles

        // Sync metadata from compiler + RPC
        self.sync_metadata_from_compiler_and_rpc();

        Ok(serde_json::json!({
            "compiled": true,
            "action_count": new_actions.len(),
            "messages_sent": messages_sent,
            "total_instructions": total_instructions
        }))
    }

    /// Sync metadata by querying the current tip from RPC and looking it up in compiler metadata
    fn sync_metadata_from_compiler_and_rpc(&mut self) {
        let compiler = match self.compiler.as_ref() {
            Some(c) => c,
            None => return,
        };

        // Get RPC client
        let rpc_client = match std::env::var("NODE1_RPC_URL")
            .ok()
            .and_then(|url| RpcClient::from_url(&url).ok())
        {
            Some(c) => c,
            None => return,
        };

        // Query current tip from RPC
        let tip_hash = match rpc_client.get_best_block_hash() {
            Ok(h) => h,
            Err(_) => return,
        };

        let tip_info = match rpc_client.get_block_header(&tip_hash) {
            Ok(info) => info,
            Err(_) => return,
        };

        // Parse the block hash and look it up in compiler metadata
        let block_hash: bitcoin::BlockHash = match tip_hash.parse() {
            Ok(h) => h,
            Err(_) => return,
        };

        let compiler_metadata = compiler.metadata();

        // If the tip block was compiled by us, update metadata
        if let Some((header_var_idx, _block_var_idx, _tx_vars)) =
            compiler_metadata.block_variables(&block_hash)
        {
            let instr_idx = compiler_metadata
                .variable_instruction(header_var_idx)
                .unwrap_or(0);

            let recent_block = RecentBlock {
                height: tip_info.height,
                defining_block: (header_var_idx, instr_idx),
            };

            self.current_tip_height = tip_info.height;
            self.metadata.add_recent_blocks(vec![recent_block]);
        }
    }
}

fn handle_request(state: &mut IrBuilderState, request: Request) -> Response {
    match request.method.as_str() {
        "init" => match state.init(request.params) {
            Ok(result) => Response::success(result),
            Err(e) => Response::error(&e),
        },

        "mutate" => match state.mutate(request.params) {
            Ok(result) => Response::success(result),
            Err(e) => Response::error(&e),
        },

        "compile" => match state.compile(request.params) {
            Ok(result) => Response::success(result),
            Err(e) => Response::error(&e),
        },

        _ => Response::error(&format!("unknown method: {}", request.method)),
    }
}

fn handle_client(stream: TcpStream, state: Arc<Mutex<IrBuilderState>>) {
    let peer_addr = stream
        .peer_addr()
        .unwrap_or_else(|_| "unknown".parse().unwrap());
    let mut reader = BufReader::new(stream.try_clone().expect("failed to clone stream"));
    let mut writer = stream;

    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                break;
            }
            Ok(_) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let response = match serde_json::from_str::<Request>(line) {
                    Ok(request) => {
                        let mut state = state.lock().unwrap();
                        handle_request(&mut state, request)
                    }
                    Err(e) => Response::error(&format!("invalid request: {}", e)),
                };

                let response_json =
                    serde_json::to_string(&response).expect("failed to serialize response");
                if let Err(e) = writeln!(writer, "{}", response_json) {
                    println!("ir-builder: failed to send response: {}", e);
                    break;
                }
                let _ = writer.flush();
            }
            Err(e) => {
                println!("ir-builder: read error: {}", e);
                break;
            }
        }
    }
}

fn main() {
    let port = std::env::var("IR_BUILDER_PORT").unwrap_or_else(|_| "9000".to_string());
    let addr = format!("0.0.0.0:{}", port);

    println!("ir-builder: starting TCP server on {}", addr);

    let listener = TcpListener::bind(&addr).expect("failed to bind");

    println!("ir-builder: listening on {}", addr);

    // Create shared state
    let state = Arc::new(Mutex::new(IrBuilderState::new()));

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || {
                    handle_client(stream, state);
                });
            }
            Err(e) => {
                println!("ir-builder: accept error: {}", e);
            }
        }
    }
}
