//! Bounded fixture collector exchanges shared by Image and provided ZIP handlers.

use std::{env, io::Read, net::SocketAddr};

use serde_json::Value;
use ureq::Agent;

type Result<T> = std::result::Result<T, &'static str>;

pub(super) fn variable(name: &str) -> Result<String> {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or("missing or oversized fixture environment")
}
pub(super) fn barrier(agent: &Agent, phase: &str, body: &Value) -> Result<Value> {
    let address: SocketAddr = variable("IMAGE_FIXTURE_COLLECTOR")?
        .parse()
        .map_err(|_| "invalid fixture collector")?;
    if !(address.ip().is_loopback()
        || matches!(address.ip(), std::net::IpAddr::V4(ip) if ip.is_private()))
        || address.port() == 0
    {
        return Err("fixture collector is not an explicit local address");
    }
    let token = variable("IMAGE_FIXTURE_TOKEN")?;
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid fixture collector token");
    }
    let response = agent
        .post(&format!("http://{address}/{phase}"))
        .set("x-fixture-token", &token)
        .send_string(&body.to_string())
        .map_err(|_| "fixture collector exchange failed")?;
    let mut raw = Vec::new();
    response
        .into_reader()
        .take(2049)
        .read_to_end(&mut raw)
        .map_err(|_| "fixture collector response failed")?;
    if raw.len() > 2048 {
        return Err("fixture collector response exceeds limit");
    }
    serde_json::from_slice(&raw).map_err(|_| "fixture collector response is not JSON")
}
