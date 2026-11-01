//! Linux process wiring with nonblocking owned pipe descriptors.

use std::{
    env,
    io::{stdin, stdout},
    os::fd::AsFd,
    time::Duration,
};

use rustack_lambda_runtime::{
    http::start_at,
    protocol::{Architecture, EnvironmentId},
    relay::{RelayConfig, RelayError, start},
};
use tokio::net::unix::pipe::{Receiver, Sender};

fn seconds(key: &str, default: u64, maximum: u64) -> Result<Duration, RelayError> {
    let value = match env::var(key) {
        Ok(value) => {
            if value.len() > 5 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(RelayError::Configuration);
            }
            let parsed = value
                .parse::<u64>()
                .map_err(|_| RelayError::Configuration)?;
            if parsed.to_string() != value {
                return Err(RelayError::Configuration);
            }
            parsed
        }
        Err(env::VarError::NotPresent) => default,
        Err(_) => return Err(RelayError::Configuration),
    };
    if value == 0 || value > maximum {
        return Err(RelayError::Configuration);
    }
    Ok(Duration::from_secs(value))
}

/// Run the private Linux relay process.
pub(crate) async fn run() -> Result<(), RelayError> {
    let mut arguments = env::args_os();
    arguments.next();
    match arguments.next() {
        None => {}
        Some(command) if command == "probe" && arguments.next().is_none() => {
            return rustack_lambda_runtime::probe::run()
                .await
                .map_err(|_| RelayError::Configuration);
        }
        Some(command) if command == "endpoint-probe" && arguments.next().is_none() => {
            return rustack_lambda_runtime::probe::run_endpoint()
                .await
                .map_err(|_| RelayError::Configuration);
        }
        Some(_) => return Err(RelayError::Configuration),
    }
    let environment =
        env::var("RUSTACK_LAMBDA_ENVIRONMENT_ID").map_err(|_| RelayError::Configuration)?;
    if environment.len() != 36 {
        return Err(RelayError::Configuration);
    }
    let environment: EnvironmentId = environment.parse().map_err(|_| RelayError::Configuration)?;
    let architecture = match env::consts::ARCH {
        "x86_64" => Architecture::X86_64,
        "aarch64" => Architecture::Arm64,
        _ => return Err(RelayError::UnsupportedPlatform),
    };
    let idle = seconds("RUSTACK_LAMBDA_RELAY_IDLE_SECONDS", 600, 86_400)?;
    let io = seconds("RUSTACK_LAMBDA_RELAY_IO_SECONDS", 5, 60)?;
    let retirement = seconds("RUSTACK_LAMBDA_RELAY_RETIRE_SECONDS", 10, 60)?;
    let config = RelayConfig::new(environment, architecture, idle, io, retirement)?;
    let input = Receiver::from_owned_fd(stdin().as_fd().try_clone_to_owned()?)?;
    let output = Sender::from_owned_fd(stdout().as_fd().try_clone_to_owned()?)?;
    let api = start_at(
        "127.0.0.1:9001"
            .parse()
            .map_err(|_| RelayError::Configuration)?,
        idle,
    )
    .await?;
    let mut relay = start(api, config, input, output)?;
    relay.wait().await
}
