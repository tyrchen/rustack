//! Optional per-process observation for identical Docker/Native ZIP fixtures.

#[path = "fixture_collector.rs"]
mod collector;

use std::{
    env,
    os::unix::fs::{MetadataExt, PermissionsExt},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    fs,
    io::AsyncWriteExt,
    runtime::{Builder, Runtime},
    time::timeout,
};

/// One selected fixture process and its bounded observation runtime.
#[derive(Debug)]
pub struct Probe {
    runtime: Runtime,
    calls: u64,
}
impl Probe {
    /// Select observations only when explicitly enabled by the fixture.
    pub fn selected() -> Result<Option<Self>, &'static str> {
        if env::var("PROVIDED_QUALIFICATION").as_deref() != Ok("1") {
            return Ok(None);
        }
        if env::var("PROVIDED_HOLD_STAGE").as_deref() == Ok("initializing") {
            hold("initializing", None)?;
        }
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "probe runtime")?;
        Ok(Some(Self { runtime, calls: 0 }))
    }
    /// Add computed per-process/sandbox evidence without changing normal fixtures.
    pub fn augment(
        &mut self,
        event: &Value,
        response: &mut Value,
        request_id: &str,
    ) -> Result<(), &'static str> {
        if event.get("providedAction").and_then(Value::as_str) == Some("exit") {
            std::process::exit(23);
        }
        if env::var("PROVIDED_HOLD_STAGE").as_deref() == Ok("busy") {
            hold("busy", Some(request_id))?;
        }
        self.calls = self.calls.checked_add(1).ok_or("probe counter limit")?;
        let snapshot = self
            .runtime
            .block_on(async { timeout(Duration::from_secs(5), snapshot(self.calls)).await })
            .map_err(|_| "probe observation expired")??;
        let object = response.as_object_mut().ok_or("probe response shape")?;
        object.insert("providedCounter".into(), json!(self.calls));
        object.insert("providedSandbox".into(), snapshot);
        Ok(())
    }
}
async fn snapshot(calls: u64) -> Result<Value, &'static str> {
    let metadata = fs::metadata("bootstrap")
        .await
        .map_err(|_| "probe bootstrap metadata")?;
    let mode = metadata.mode() & 0o777;
    let mut permissions = metadata.permissions();
    permissions.set_mode(0o755);
    let read_only = match fs::set_permissions("bootstrap", permissions).await {
        Ok(()) => false,
        Err(error) if error.raw_os_error() == Some(30) => true,
        Err(_) => return Err("probe chmod failed without EROFS"),
    };
    let name = format!("/tmp/rustack-provided-{}-{calls}", std::process::id());
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&name)
        .await
        .map_err(|_| "probe temporary create")?;
    file.write_all(b"ok")
        .await
        .map_err(|_| "probe temporary write")?;
    file.flush().await.map_err(|_| "probe temporary flush")?;
    drop(file);
    let data = fs::read(&name).await.map_err(|_| "probe temporary read")?;
    fs::remove_file(&name)
        .await
        .map_err(|_| "probe temporary removal")?;
    if data != b"ok" {
        return Err("probe temporary contents");
    }
    Ok(
        json!({"codeReadOnly":read_only,"codeUid":metadata.uid(),"codeGid":metadata.gid(),"codeMode":mode,"tmpWritable":true}),
    )
}

fn hold(stage: &str, id: Option<&str>) -> Result<(), &'static str> {
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .timeout(Duration::from_secs(240))
        .build();
    let name = collector::variable("AWS_LAMBDA_FUNCTION_NAME")?;
    let value = collector::barrier(
        &agent,
        "hold",
        &json!({"stage":stage,"functionName":name,"pid":std::process::id(),"requestId":id}),
    )?;
    if value.get("release") != Some(&Value::Bool(true)) {
        return Err("provided fixture hold not released");
    }
    Ok(())
}
