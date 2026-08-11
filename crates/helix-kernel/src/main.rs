use std::error::Error;
use std::io::{self, Write};
use std::sync::Arc;

use helix_ipc::{
    KERNEL_EPOCH_ENV, KERNEL_LAUNCH_TOKEN_ENV, KERNEL_READY_PREFIX, KernelReady,
    serve_internal_rpc_request,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let launch_token = std::env::var(KERNEL_LAUNCH_TOKEN_ENV)
        .map_err(|_| format!("{KERNEL_LAUNCH_TOKEN_ENV} is required"))?;
    let epoch =
        std::env::var(KERNEL_EPOCH_ENV).map_err(|_| format!("{KERNEL_EPOCH_ENV} is required"))?;
    let kernel = helix_kernel_lib::bootstrap().await?;
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let ready = KernelReady {
        port: listener.local_addr()?.port(),
        process_id: std::process::id(),
        epoch: epoch.clone(),
    };
    println!("{KERNEL_READY_PREFIX}{}", serde_json::to_string(&ready)?);
    io::stdout().flush()?;

    let dispatcher = kernel.dispatcher.clone();
    let launch_token = Arc::new(launch_token);
    let epoch = Arc::new(epoch);
    loop {
        let (stream, _) = listener.accept().await?;
        let dispatcher = dispatcher.clone();
        let launch_token = launch_token.clone();
        let epoch = epoch.clone();
        tokio::spawn(async move {
            if let Err(error) =
                serve_internal_rpc_request(stream, &launch_token, &epoch, dispatcher).await
            {
                eprintln!("internal RPC request failed: {error}");
            }
        });
    }
}
