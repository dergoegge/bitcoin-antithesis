use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

pub use fuzzamoto_ir::{FullProgramContext, ProgramContext};
pub use fuzzamoto_ir::generators::{Header, Txo};
use jsonrpc::simple_http::SimpleHttpTransport;
use jsonrpc::Client as JsonRpcClient;
use serde::Deserialize;
use serde_json::value::RawValue;

/// Bitcoin RPC client wrapper
pub struct Client {
    inner: JsonRpcClient,
}

impl Client {
    pub fn new(url: &str, user: &str, password: &str) -> Result<Self, String> {
        let transport = SimpleHttpTransport::builder()
            .url(url)
            .map_err(|e| format!("invalid url: {}", e))?
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
        let url =
            env::var(&env_var).unwrap_or_else(|_| panic!("Missing environment variable: {}", env_var));

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

/// Get all node configurations from environment (2 nodes for ir-workload)
pub fn get_all_nodes() -> Vec<NodeConfig> {
    vec![NodeConfig::from_env("NODE1"), NodeConfig::from_env("NODE2")]
}

/// Create an RPC client for a node
pub fn create_client(config: &NodeConfig) -> Result<Client, String> {
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

/// IR Builder client configuration
pub struct IrBuilderConfig {
    pub host: String,
    pub port: u16,
}

impl IrBuilderConfig {
    pub fn from_env() -> Self {
        let host = env::var("IR_BUILDER_HOST").unwrap_or_else(|_| "ir-builder".to_string());
        let port: u16 = env::var("IR_BUILDER_PORT")
            .unwrap_or_else(|_| "9000".to_string())
            .parse()
            .expect("Invalid IR_BUILDER_PORT");

        Self { host, port }
    }

    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// IR Builder client for making requests to the ir-builder TCP server
pub struct IrBuilderClient {
    config: IrBuilderConfig,
}

#[derive(Debug, Deserialize)]
pub struct IrBuilderResponse {
    pub success: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl IrBuilderClient {
    pub fn new(config: IrBuilderConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Self {
        Self::new(IrBuilderConfig::from_env())
    }

    /// Send a request to the ir-builder and get a response
    pub fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<IrBuilderResponse, std::io::Error> {
        let mut stream = TcpStream::connect(self.config.address())?;

        let request = serde_json::json!({
            "method": method,
            "params": params
        });

        writeln!(stream, "{}", serde_json::to_string(&request).unwrap())?;
        stream.flush()?;

        let mut reader = BufReader::new(&stream);
        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;

        let response: IrBuilderResponse = serde_json::from_str(&response_line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        Ok(response)
    }

    /// Initialize the ir-builder with the full program context
    pub fn init(&self, context: &FullProgramContext) -> Result<IrBuilderResponse, std::io::Error> {
        let params = serde_json::to_value(context)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.call("init", params)
    }

    /// Request a mutation from ir-builder
    pub fn mutate(&self, params: serde_json::Value) -> Result<IrBuilderResponse, std::io::Error> {
        self.call("mutate", params)
    }

    /// Request a compilation from ir-builder
    pub fn compile(&self, params: serde_json::Value) -> Result<IrBuilderResponse, std::io::Error> {
        self.call("compile", params)
    }

}

/// Macro to generate a mutate driver main function
#[macro_export]
macro_rules! mutate_driver {
    ($mutator_type:expr, $message:literal) => {
        fn main() {
            let ir_client = ir_workload::IrBuilderClient::from_env();
            match ir_client.mutate(serde_json::json!({ "type": $mutator_type })) {
                Ok(response) => {
                    antithesis_sdk::assert_sometimes!(
                        response.success,
                        $message,
                        &serde_json::json!({ "result": response.result })
                    );
                }
                Err(_) => {}
            }
        }
    };
}
