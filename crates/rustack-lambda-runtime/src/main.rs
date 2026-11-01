//! Linux-only Lambda relay process. Stdout is reserved for binary control frames.

#![forbid(unsafe_code)]

#[cfg(not(target_os = "linux"))]
use std::future::{Ready, ready};
use std::process::ExitCode;

#[cfg(not(target_os = "linux"))]
use rustack_lambda_runtime::relay::RelayError;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_writer(std::io::stderr)
        .finish();
    if tracing::subscriber::set_global_default(subscriber).is_err() {
        return ExitCode::FAILURE;
    }
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "Lambda runtime relay exited");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn run() -> Ready<Result<(), RelayError>> {
    ready(Err(RelayError::UnsupportedPlatform))
}

#[cfg(target_os = "linux")]
mod process;
#[cfg(target_os = "linux")]
use process::run;
