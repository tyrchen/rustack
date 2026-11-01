//! One retained owner above every CLI/SDK observer and all daemon effects.

#[path = "owner/cleanup.rs"]
mod cleanup;
#[path = "owner/configuration.rs"]
mod configuration;
#[path = "owner/data.rs"]
mod data;

use std::{
    fs::Permissions,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::docker::models::{ContainerInspectResponse, NetworkInspect, Volume};
use serde::Deserialize;
use serde_json::json;
use tempfile::TempDir;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    time::{Instant, sleep, timeout_at},
};

use self::data::{DataOperation, DataOutput, commit_identity};
use super::{
    ImageProfile, Mode,
    cli::{self, Cli, arguments},
    policy::{self, Expected, Inputs},
    records::Identity,
};

const INSPECT: &str = r#"{"Id":{{json .Id}},"Name":{{json .Name}},"Image":{{json .Image}},"State":{{json .State}},"Config":{"Image":{{json .Config.Image}},"User":{{json .Config.User}},"WorkingDir":{{json .Config.WorkingDir}},"Entrypoint":{{json .Config.Entrypoint}},"Cmd":{{json .Config.Cmd}},"Labels":{{json .Config.Labels}},"Healthcheck":{{json .Config.Healthcheck}}},"HostConfig":{{json .HostConfig}},"NetworkSettings":{{json .NetworkSettings}},"Mounts":{{json .Mounts}}}"#;
const SOURCE: &str =
    r#"{"id":{{json .Id}},"os":{{json .Os}},"architecture":{{json .Architecture}}}"#;

#[derive(Debug)]
enum Resources {
    Intent,
    Volume(Volume),
    Server { volume: Volume, id: String },
}
#[derive(Debug, Clone, Copy)]
enum ExpectedExit {
    Success,
    EndpointRejected(i64),
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    id: String,
    os: String,
    architecture: String,
}
#[derive(Debug)]
pub(super) struct Owner {
    root: TempDir,
    profile: ImageProfile,
    path: PathBuf,
    pub cli: Cli,
    pub inputs: Inputs,
    pub expected: Expected,
    resources: Resources,
    uncertain: bool,
    identity: Option<Identity>,
    endpoint: Option<String>,
    expected_exit: ExpectedExit,
    sources: Vec<(String, String)>,
}

impl Owner {
    pub async fn new(mode: Mode, profile: ImageProfile, cutoff: Instant) -> Result<Self> {
        let inputs = Inputs::load(profile)?;
        let root = tempfile::tempdir()?;
        timeout_at(
            cutoff,
            fs::set_permissions(root.path(), Permissions::from_mode(0o700)),
        )
        .await??;
        let path = timeout_at(cutoff, fs::canonicalize(root.path())).await??;
        let visible = path.to_str().context("private evidence path encoding")?;
        ensure!(
            visible.len() <= 4096 && !visible.chars().any(char::is_control),
            "private evidence path bound"
        );
        let cli = Cli::connect(inputs.program.clone(), inputs.client.clone(), cutoff).await?;
        let [network]: [NetworkInspect; 1] = cli
            .json(&arguments(&["network", "inspect", &inputs.network]), cutoff)
            .await?;
        let network = policy::network(&network, &inputs.network)?;
        let mut sources = Vec::new();
        for reference in [&inputs.image, &inputs.sdk_image, &inputs.relay] {
            let value: Source = cli
                .json(
                    &arguments(&["image", "inspect", "--format", SOURCE, reference]),
                    cutoff,
                )
                .await?;
            ensure!(
                value.os == "linux" && value.architecture == inputs.architecture,
                "server fixture source platform differs"
            );
            sources.push((reference.clone(), policy::source_id(&value.id)?));
        }
        let source = sources.first().context("server source capture")?.1.clone();
        profile.source(&cli, &source, cutoff).await?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let expected = Expected {
            name: format!("rustack-server-{nonce}"),
            volume: format!("rustack-server-data-{nonce}"),
            nonce,
            engine: cli.engine,
            source,
            network,
            socket: inputs.source_socket.clone(),
            gid: inputs.gid.clone(),
        };
        let mut owner = Self {
            root,
            profile,
            path,
            cli,
            inputs,
            expected,
            resources: Resources::Intent,
            uncertain: false,
            identity: None,
            endpoint: None,
            expected_exit: ExpectedExit::Success,
            sources,
        };
        owner.write_environment(mode, cutoff).await?;
        owner.write("intent.json", &serde_json::to_vec(&json!({
            "schemaVersion":1,"engine":owner.expected.engine.to_string(),"nonce":owner.expected.nonce,
            "server":owner.expected.name,"volume":owner.expected.volume,"source":owner.expected.source,
            "network":owner.expected.network,"daemonSocket":owner.expected.socket,"socketGid":owner.expected.gid,
            "sources":owner.sources
        }))?, cutoff).await?;
        owner.root.disable_cleanup(true);
        eprintln!("server-container evidence: {}", owner.path.display());
        Ok(owner)
    }

    pub async fn prepare(&mut self, cutoff: Instant) -> Result<()> {
        let mut args = arguments(&["volume", "create", "--driver", "local"]);
        for (key, value) in self.expected.labels("server-data") {
            args.extend(["--label".into(), format!("{key}={value}")]);
        }
        args.push(self.expected.volume.clone());
        self.uncertain = true;
        let reply = self.cli.run(&args, cutoff).await?;
        ensure!(
            reply == format!("{}\n", self.expected.volume).as_bytes(),
            "volume creation reply differs"
        );
        let data = self.inspect_volume(cutoff).await?;
        policy::volume(&data, &self.expected)?;
        self.resources = Resources::Volume(data.clone());
        self.write("volume.json", &serde_json::to_vec(&data)?, cutoff)
            .await?;
        self.uncertain = false;
        let args = self.create_arguments()?;
        self.uncertain = true;
        let reply = self.cli.run(&args, cutoff).await?;
        let text = std::str::from_utf8(&reply)?
            .strip_suffix('\n')
            .context("container creation newline")?;
        let id = cli::container_id(text)?;
        self.resources = Resources::Server {
            volume: data,
            id: id.clone(),
        };
        self.write("server.json", &serde_json::to_vec(&json!({"id":id,"engine":self.expected.engine.to_string(),
            "nonce":self.expected.nonce,"source":self.expected.source,"volume":self.expected.volume}))?, cutoff).await?;
        self.inspect_server(cutoff).await?;
        self.uncertain = false;
        if self.profile == ImageProfile::Release {
            self.install_public_configuration(cutoff).await?;
        }
        self.uncertain = true;
        let started = self.cli.run(&arguments(&["start", &id]), cutoff).await?;
        ensure!(
            started == format!("{id}\n").as_bytes(),
            "server start reply differs"
        );
        self.uncertain = false;
        Ok(())
    }

    fn create_arguments(&self) -> Result<Vec<String>> {
        let environment = self.path.join("server.env");
        let environment = environment
            .to_str()
            .context("private environment path encoding")?;
        let mut args = arguments(&[
            "create",
            "--pull=never",
            "--name",
            &self.expected.name,
            "--platform",
            &format!("linux/{}", self.inputs.architecture),
            "--user",
            "65532:65532",
            "--group-add",
            &self.expected.gid,
            "--read-only",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges:true",
            "--cgroupns",
            "private",
            "--memory",
            "512m",
            "--memory-swap",
            "512m",
            "--cpus",
            "1",
            "--pids-limit",
            "128",
            "--tmpfs",
            &format!("/tmp:{}", policy::TEMPORARY),
            "--log-driver",
            "json-file",
            "--log-opt",
            "max-size=1m",
            "--log-opt",
            "max-file=1",
            "--restart",
            "no",
            "--network",
            &self.expected.network,
            "--network-alias",
            &self.expected.name,
            "--publish",
            "127.0.0.1::4566",
            "--env-file",
            environment,
            "--mount",
            &format!(
                "type=bind,source={},target=/run/rustack/docker.sock,readonly",
                self.expected.socket
            ),
            "--mount",
            &format!("type=volume,source={},target=/data", self.expected.volume),
        ]);
        for (key, value) in self.expected.labels("server") {
            args.extend(["--label".into(), format!("{key}={value}")]);
        }
        args.push(self.expected.source.clone());
        Ok(args)
    }

    pub async fn ready(&mut self, cutoff: Instant) -> Result<()> {
        let at = cutoff.min(Instant::now() + Duration::from_secs(180));
        loop {
            let view = self.inspect_server(at).await?;
            ensure!(
                view.state.as_ref().and_then(|s| s.running) == Some(true),
                "server exited before readiness"
            );
            let endpoint = policy::endpoint(&view, &self.expected)?;
            self.endpoint = Some(endpoint.clone());
            if let Ok((status, value)) =
                super::super::process::get(&endpoint, "/_rustack/capabilities").await
            {
                if value.get("ready").and_then(serde_json::Value::as_bool) == Some(true)
                    && self.profile.healthy(&view)
                {
                    ensure!(
                        status == 200
                            && value
                                .pointer("/workers/docker/ready")
                                .and_then(serde_json::Value::as_bool)
                                == Some(true)
                            && value
                                .pointer("/workers/docker/sdkGateway")
                                .and_then(serde_json::Value::as_bool)
                                == Some(true)
                            && value
                                .pointer("/workflows/lambdaRequestResponse")
                                .and_then(serde_json::Value::as_bool)
                                == Some(true)
                            && value
                                .pointer("/workflows/nestedSync")
                                .and_then(serde_json::Value::as_bool)
                                == Some(true)
                            && value
                                .pointer("/workflows/stateTypes")
                                .and_then(serde_json::Value::as_array)
                                .is_some_and(|s| s.len() == 8)
                            && value
                                .pointer("/workers/docker/platform")
                                .and_then(serde_json::Value::as_str)
                                == Some(format!("linux/{}", self.inputs.architecture).as_str()),
                        "false server-container readiness"
                    );
                    ensure!(Instant::now() < at, "late server-container readiness");
                    self.capture_identity(at).await?;
                    return Ok(());
                }
            }
            ensure!(Instant::now() < at, "server-container readiness deadline");
            sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn rejection(&mut self, cutoff: Instant) -> Result<()> {
        let at = cutoff.min(Instant::now() + Duration::from_secs(180));
        loop {
            let view = self.inspect_server(at).await?;
            let state = view.state.as_ref().context("server startup state")?;
            if state.running == Some(false) {
                let code = state.exit_code.context("server startup exit code")?;
                ensure!(
                    code != 0 && state.oom_killed == Some(false),
                    "unrelated server startup exit"
                );
                let logs = self.logs(at).await?;
                let text = std::str::from_utf8(&logs)?;
                ensure!(
                    text.contains("Docker startup endpoint qualification failed"),
                    "server rejected an unrelated startup prerequisite"
                );
                self.capture_identity(at).await?;
                self.expected_exit = ExpectedExit::EndpointRejected(code);
                return Ok(());
            }
            if let Ok(endpoint) = policy::endpoint(&view, &self.expected) {
                if let Ok((_, value)) =
                    super::super::process::get(&endpoint, "/_rustack/capabilities").await
                {
                    ensure!(
                        value.get("ready").and_then(serde_json::Value::as_bool) != Some(true),
                        "unreachable server gateway became ready"
                    );
                }
            }
            ensure!(Instant::now() < at, "server gateway rejection deadline");
            sleep(Duration::from_millis(100)).await;
        }
    }

    pub fn endpoint(&self) -> Result<&str> {
        self.endpoint
            .as_deref()
            .context("qualified server endpoint")
    }
    pub fn identity(&self) -> Result<Identity> {
        self.identity
            .context("verified server installation identity")
    }
    pub fn source(&self, reference: &str) -> Result<&str> {
        self.sources
            .iter()
            .find(|(name, _)| name == reference)
            .map(|(_, id)| id.as_str())
            .context("captured source ID")
    }
    pub async fn publication_receipt(&self, mode: &str) -> Result<serde_json::Value> {
        let at = Instant::now() + Duration::from_secs(10);
        let mut documents = Vec::new();
        for name in ["server.json", "shutdown.json", "cleanup.json"] {
            let value = timeout_at(at, async {
                let file = fs::File::open(self.path.join(name)).await?;
                let mut bytes = Vec::new();
                file.take(8193).read_to_end(&mut bytes).await?;
                ensure!(bytes.len() <= 8192, "publication receipt bound");
                cli::decode::<serde_json::Value>(&bytes)
            })
            .await??;
            documents.push(value);
        }
        let mut documents = documents.into_iter();
        let server = documents.next().context("server receipt")?;
        let shutdown = documents.next().context("shutdown receipt")?;
        let cleanup = documents.next().context("cleanup receipt")?;
        Ok(json!({"schemaVersion":1,"profile":"release","mode":mode,
            "architecture":self.inputs.architecture,"server":server,"shutdown":shutdown,"cleanup":cleanup}))
    }
    pub fn release(&mut self) {
        self.root.disable_cleanup(false);
    }

    async fn inspect_volume(&self, cutoff: Instant) -> Result<Volume> {
        let [volume]: [Volume; 1] = self
            .cli
            .json(
                &arguments(&["volume", "inspect", &self.expected.volume]),
                cutoff,
            )
            .await?;
        Ok(volume)
    }
    async fn inspect_server(&self, cutoff: Instant) -> Result<ContainerInspectResponse> {
        let Resources::Server { volume, id } = &self.resources else {
            anyhow::bail!("server container not captured");
        };
        let view: ContainerInspectResponse = self
            .cli
            .json(
                &arguments(&["container", "inspect", "--format", INSPECT, id]),
                cutoff,
            )
            .await?;
        ensure!(
            view.id.as_deref() == Some(id.as_str()),
            "server inspected a different container"
        );
        policy::server(&view, &self.expected, volume)?;
        self.profile
            .health(view.config.as_ref().and_then(|c| c.healthcheck.as_ref()))?;
        Ok(view)
    }
    async fn capture_identity(&mut self, cutoff: Instant) -> Result<()> {
        self.inspect_server(cutoff).await?;
        let archives = match self.transfer_data(DataOperation::Identity, cutoff).await {
            Ok(DataOutput::Identity(archives)) => Ok(archives),
            Ok(DataOutput::Configuration(_)) => {
                Err(anyhow::anyhow!("incorrect data-volume identity operation"))
            }
            Err(error) => Err(error),
        };
        commit_identity(
            &mut self.uncertain,
            &mut self.identity,
            archives,
            self.cli.engine,
            &self.path,
            cutoff,
        )
        .await
    }
    async fn logs(&self, cutoff: Instant) -> Result<Vec<u8>> {
        let Resources::Server { id, .. } = &self.resources else {
            anyhow::bail!("server logs without container");
        };
        let output = self.cli.output(&arguments(&["logs", id]), cutoff).await?;
        let mut bytes = output.stdout;
        bytes.extend_from_slice(&output.stderr);
        super::logs::check(&bytes)?;
        Ok(bytes)
    }
    async fn write(&self, name: &str, bytes: &[u8], cutoff: Instant) -> Result<()> {
        write_private(&self.path, name, bytes, cutoff).await
    }
    async fn write_environment(&self, mode: Mode, cutoff: Instant) -> Result<()> {
        let relay = self.source(&self.inputs.relay)?;
        let configuration = self.profile.configuration();
        let text = format!(
            concat!(
                "SERVICES=lambda,stepfunctions,s3,dynamodb\n",
                "GATEWAY_LISTEN=0.0.0.0:4566\n",
                "RUSTACK_CONFIG={configuration}\n",
                "DATA_DIR=/data/state\n",
                "LAMBDA_EXECUTOR=docker\n",
                "LAMBDA_DOCKER_SOCKET=unix:///run/rustack/docker.sock\n",
                "LAMBDA_DOCKER_RELAY_IMAGE={relay}\n",
                "LAMBDA_DOCKER_NETWORK={network}\n",
                "LAMBDA_DOCKER_GATEWAY_ENDPOINT=http://{server}:{port}/\n",
                "LAMBDA_DOCKER_CROSS_ARCHITECTURE=false\n",
                "LAMBDA_INIT_TIMEOUT_SECS=20\n",
                "LOG_LEVEL=error\n",
                "LAMBDA_SKIP_SIGNATURE_VALIDATION=false\n",
                "STEPFUNCTIONS_SKIP_SIGNATURE_VALIDATION=false\n",
                "S3_SKIP_SIGNATURE_VALIDATION=false\n",
                "DYNAMODB_SKIP_SIGNATURE_VALIDATION=false\n",
                "ACCESS_KEY=local-key\n",
                "SECRET_KEY=local-secret\n",
                "AWS_ACCESS_KEY_ID=local-key\n",
                "AWS_SECRET_ACCESS_KEY=local-secret\n",
                "AWS_EC2_METADATA_DISABLED=true\n",
                "LAMBDA_DOCKER_SDK_ACCESS_KEY_ID=local-key\n",
                "LAMBDA_DOCKER_SDK_SECRET_ACCESS_KEY=local-secret\n",
            ),
            configuration = configuration,
            relay = relay,
            network = self.expected.network,
            server = self.expected.name,
            port = mode.gateway_port(),
        );
        self.write("server.env", text.as_bytes(), cutoff).await
    }
}

async fn write_private(directory: &Path, name: &str, bytes: &[u8], cutoff: Instant) -> Result<()> {
    let at = cutoff.min(Instant::now() + Duration::from_secs(10));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = timeout_at(at, options.open(directory.join(name))).await??;
    timeout_at(at, file.write_all(bytes)).await??;
    timeout_at(at, file.sync_all()).await??;
    ensure!(Instant::now() < at, "private server evidence write expired");
    Ok(())
}
