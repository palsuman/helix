use std::error::Error;
use std::io::{self, Write};
use std::sync::Arc;

use helix_ipc::{
    KERNEL_CRASH_HANDOFF_ENV, KERNEL_EPOCH_ENV, KERNEL_LAUNCH_TOKEN_ENV, KERNEL_READY_PREFIX,
    KernelReady, serve_internal_rpc_request_with_shutdown,
};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    install_panic_handoff(None);
    let launch_token = std::env::var(KERNEL_LAUNCH_TOKEN_ENV)
        .map_err(|_| format!("{KERNEL_LAUNCH_TOKEN_ENV} is required"))?;
    let epoch =
        std::env::var(KERNEL_EPOCH_ENV).map_err(|_| format!("{KERNEL_EPOCH_ENV} is required"))?;
    let mut kernel = helix_kernel_lib::bootstrap().await?;
    install_panic_handoff(Some(kernel.logger.clone()));
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
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel(1);
    loop {
        let stream = tokio::select! {
            accepted = listener.accept() => accepted?.0,
            requested = shutdown_rx.recv() => {
                if requested.is_some() {
                    kernel.container.stop_all().await?;
                    return Ok(());
                }
                continue;
            }
        };
        let dispatcher = dispatcher.clone();
        let launch_token = launch_token.clone();
        let epoch = epoch.clone();
        let shutdown_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_internal_rpc_request_with_shutdown(
                stream,
                &launch_token,
                &epoch,
                dispatcher,
                Some(shutdown_tx),
            )
            .await
            {
                eprintln!("internal RPC request failed: {error}");
            }
        });
    }
}

fn install_panic_handoff(logger: Option<Arc<helix_log::Logger>>) {
    let Some(path) = std::env::var_os(KERNEL_CRASH_HANDOFF_ENV).map(std::path::PathBuf::from)
    else {
        return;
    };
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|value| (*value).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic payload".into());
        let message = match &logger {
            Some(logger) => logger.redactor().redact_text(&message),
            None => helix_log::Redactor::new().redact_text(&message),
        };
        let location = info.location().map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        });
        let record = serde_json::json!({ "message": message, "location": location });
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, record.to_string());
    }));
}
