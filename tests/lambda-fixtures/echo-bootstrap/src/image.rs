//! Independent bounded custom Rust Runtime Interface Client for real Image tests.
#![forbid(unsafe_code)]

use std::{
    env, hint::black_box, io::Read, net::SocketAddr, path::Path, process, thread, time::Duration,
};

use serde_json::{Value, json};
use ureq::{Agent, AgentBuilder, Error};

const EVENT_BYTES: u64 = 65_536;
type Result<T> = std::result::Result<T, &'static str>;

fn variable(name: &str) -> Result<String> {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or("missing or oversized fixture environment")
}
fn request_id(value: &str) -> Result<&str> {
    if value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        Ok(value)
    } else {
        Err("invalid fixture request identity")
    }
}
fn post(agent: &Agent, root: &str, path: &str, body: &str) -> Result<u16> {
    match agent.post(&format!("{root}/{path}")).send_string(body) {
        Ok(response) => Ok(response.status()),
        Err(Error::Status(status, _)) => Ok(status),
        Err(_) => Err("fixture Runtime API transport failed"),
    }
}
fn next(agent: &Agent, root: &str) -> Result<(String, String, Value)> {
    let response = agent
        .get(&format!("{root}/invocation/next"))
        .call()
        .map_err(|_| "fixture next invocation failed")?;
    let id = request_id(
        response
            .header("lambda-runtime-aws-request-id")
            .ok_or("missing request identity")?,
    )?
    .to_owned();
    let arn = response
        .header("lambda-runtime-invoked-function-arn")
        .filter(|value| value.len() <= 2048)
        .ok_or("missing invoked ARN")?
        .to_owned();
    let mut body = Vec::new();
    response
        .into_reader()
        .take(EVENT_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| "fixture event read failed")?;
    if u64::try_from(body.len()).map_err(|_| "fixture event length")? > EVENT_BYTES {
        return Err("fixture event exceeds limit");
    }
    let event = serde_json::from_slice(&body).map_err(|_| "fixture event is not JSON")?;
    Ok((id, arn, event))
}
fn forged(agent: &Agent, root: &str, event: &Value, previous: Option<&str>) -> Result<Vec<u16>> {
    let foreign = event
        .get("foreignRequestId")
        .and_then(Value::as_str)
        .unwrap_or("11111111-1111-4111-8111-111111111111");
    request_id(foreign)?;
    let mut statuses = Vec::new();
    for id in [Some(foreign), Some("malformed"), previous]
        .into_iter()
        .flatten()
    {
        let status = post(
            agent,
            root,
            &format!("invocation/{id}/response"),
            r#"{"forged":true}"#,
        )?;
        if status != 400 {
            return Err("fixture foreign/stale completion was not rejected");
        }
        statuses.push(status);
    }
    Ok(statuses)
}
fn barrier(agent: &Agent, phase: &str, body: &Value) -> Result<Value> {
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
fn hold(stage: &str, id: Option<&str>) -> Result<()> {
    let agent = http_agent(Duration::from_secs(240));
    let name = variable("AWS_LAMBDA_FUNCTION_NAME")?;
    let release = barrier(
        &agent,
        "hold",
        &json!({"stage":stage,"functionName":name,"pid":process::id(),"requestId":id}),
    )?;
    if release.get("release") != Some(&Value::Bool(true)) {
        return Err("fixture hold was not released");
    }
    Ok(())
}
fn initialize(agent: &Agent, root: &str) -> Result<()> {
    let mode = match env::var("IMAGE_FIXTURE_INIT_MODE") {
        Ok(mode) => mode,
        Err(env::VarError::NotPresent) => return Ok(()),
        Err(_) => return Err("invalid fixture init mode"),
    };
    match mode.as_str() {
        "error" => {
            if post(
                agent,
                root,
                "init/error",
                r#"{"errorType":"FixtureInitError","errorMessage":"fixture-init-payload-sentinel"}"#,
            )? != 202
            {
                return Err("fixture initialization error was not accepted");
            }
            thread::sleep(Duration::from_secs(30));
            Err("failed-init fixture was not retired")
        }
        "exit" => process::exit(42),
        "stall" => {
            thread::sleep(Duration::from_secs(30));
            Err("stalled-init fixture was not retired")
        }
        "hold" => hold("initializing", None),
        _ => Err("unsupported fixture init mode"),
    }
}
fn exhaust_cgroup_memory() -> Result<()> {
    let mut memory = Vec::new();
    memory
        .try_reserve_exact(256 * 1024 * 1024)
        .map_err(|_| "fixture virtual allocation failed")?;
    let block = [0x5a_u8; 8 * 1024];
    for _ in 0..32768 {
        memory.extend_from_slice(&block);
        black_box(memory.as_slice());
    }
    Err("fixture unexpectedly survived its cgroup memory limit")
}
fn execute(agent: &Agent, root: &str, image: &str, arguments: &[String]) -> Result<()> {
    let mut counter = 0_u32;
    let mut previous = None::<String>;
    loop {
        let (id, arn, event) = next(agent, root)?;
        counter = counter
            .checked_add(1)
            .filter(|value| *value <= 10_000)
            .ok_or("fixture invocation limit")?;
        let value = event.get("value").and_then(Value::as_i64).unwrap_or(0);
        let computed = value.checked_add(7).ok_or("fixture arithmetic overflow")?;
        let delay = event
            .get("delayMillis")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if delay > 15_000 {
            return Err("fixture delay exceeds limit");
        }
        let mode = event
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("compute");
        let statuses = match mode {
            "compute" | "error" => Vec::new(),
            "hold" => {
                hold("busy", Some(&id))?;
                Vec::new()
            }
            "exit" => process::exit(42),
            "oom" => {
                exhaust_cgroup_memory()?;
                return Err("OOM fixture was not killed");
            }
            "forge" => forged(agent, root, &event, previous.as_deref())?,
            "barrier-forge" => {
                let lane = variable("IMAGE_FIXTURE_LANE")?;
                let peer = barrier(agent, "consume", &json!({"lane":lane,"requestId":id}))?;
                let statuses = forged(agent, root, &peer, previous.as_deref())?;
                let release = barrier(
                    agent,
                    "checked",
                    &json!({"lane":lane,"requestId":id,"statuses":statuses}),
                )?;
                if release.get("release") != Some(&Value::Bool(true)) {
                    return Err("fixture barrier did not release");
                }
                statuses
            }
            _ => return Err("unsupported fixture mode"),
        };
        thread::sleep(Duration::from_millis(delay));
        let (suffix, output) = if mode == "error" {
            (
                "error",
                json!({"errorType":"ImageFixtureError","errorMessage":"deliberate image failure"}),
            )
        } else {
            let cwd = env::current_dir().map_err(|_| "fixture cwd unavailable")?;
            (
                "response",
                json!({"computed":computed,"counter":counter,"imageRevision":image,
                "requestId":id,"invokedArn":arn,"arguments":arguments,"cwd":cwd,
                "forgedStatuses":statuses,"shellSideEffect":Path::new("/tmp/rustack-shell-side-effect").exists()}),
            )
        };
        let output = serde_json::to_string(&output).map_err(|_| "fixture response encoding")?;
        if post(agent, root, &format!("invocation/{id}/{suffix}"), &output)? != 202 {
            return Err("fixture valid completion was not accepted");
        }
        previous = Some(id);
    }
}
fn http_agent(timeout: Duration) -> Agent {
    AgentBuilder::new()
        .try_proxy_from_env(false)
        .timeout(timeout)
        .build()
}
fn main() -> Result<()> {
    let address: SocketAddr = variable("AWS_LAMBDA_RUNTIME_API")?
        .parse()
        .map_err(|_| "invalid fixture runtime endpoint")?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("fixture endpoint is not loopback");
    }
    let image = variable("IMAGE_FIXTURE_REVISION")?;
    let arguments: Vec<_> = env::args().take(17).collect();
    if arguments.len() > 16 || arguments.iter().any(|arg| arg.len() > 4096) {
        return Err("fixture argv exceeds limit");
    }
    let agent = http_agent(Duration::from_secs(120));
    let root = format!("http://{address}/2018-06-01/runtime");
    initialize(&agent, &root)?;
    execute(&agent, &root, &image, &arguments)
}
