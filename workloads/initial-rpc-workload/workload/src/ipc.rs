use std::path::Path;

use bitcoin_capnp_types::{
    echo_capnp::echo, init_capnp::init, mining_capnp::mining, proxy_capnp::thread,
};
use capnp_rpc::{rpc_twoparty_capnp::Side, twoparty::VatNetwork, RpcSystem};
use futures::io::BufReader;
use tokio::net::{unix::OwnedReadHalf, UnixStream};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

/// Connect to a Unix socket, create RPC system, and bootstrap the Init client.
/// The RPC system is spawned as a local task so that RPC calls can proceed.
///
/// Returns (init_client) - ready for making construct/echo/mining requests.
async fn connect_init(
    socket_path: impl AsRef<Path>,
) -> Result<init::Client, Box<dyn std::error::Error>> {
    let unix_stream = UnixStream::connect(socket_path).await?;
    let (reader, writer) = unix_stream.into_split();
    let buf_reader = BufReader::new(reader.compat());
    let buf_writer = futures::io::BufWriter::new(writer.compat_write());
    let network: VatNetwork<BufReader<Compat<OwnedReadHalf>>> =
        VatNetwork::new(buf_reader, buf_writer, Side::Client, Default::default());

    let mut rpc_system = RpcSystem::new(Box::new(network), None);
    let client: init::Client = rpc_system.bootstrap(Side::Server);

    // Spawn the RPC system so that requests can be processed
    tokio::task::spawn_local(rpc_system);

    Ok(client)
}

/// Full bootstrap: connect to socket, bootstrap Init client, construct ThreadMap,
/// and create a Thread. The RPC system is spawned as a local task internally.
///
/// Must be called within a `tokio::task::LocalSet` context.
///
/// Returns (init_client, thread).
pub async fn bootstrap(
    socket_path: impl AsRef<Path>,
) -> Result<(init::Client, thread::Client), Box<dyn std::error::Error>> {
    let client = connect_init(socket_path).await?;

    // Construct to get ThreadMap
    let construct_response = client.construct_request().send().promise.await?;
    let thread_map = construct_response.get()?.get_thread_map()?;

    // Create a thread
    let thread_response = thread_map.make_thread_request().send().promise.await?;
    let thread: thread::Client = thread_response.get()?.get_result()?;

    Ok((client, thread))
}

/// Get an Echo client from an Init client.
pub async fn make_echo(
    init_client: &init::Client,
    thread: &thread::Client,
) -> Result<echo::Client, Box<dyn std::error::Error>> {
    let mut req = init_client.make_echo_request();
    req.get().get_context()?.set_thread(thread.clone());
    let response = req.send().promise.await?;
    Ok(response.get()?.get_result()?)
}

/// Get a Mining client from an Init client.
pub async fn make_mining(
    init_client: &init::Client,
    thread: &thread::Client,
) -> Result<mining::Client, Box<dyn std::error::Error>> {
    let mut req = init_client.make_mining_request();
    req.get().get_context()?.set_thread(thread.clone());
    let response = req.send().promise.await?;
    Ok(response.get()?.get_result()?)
}
