//! Independent inspected server policy and exact mutation authority.

#[cfg(test)]
#[path = "policy/tests.rs"]
mod tests;

use std::{collections::BTreeMap, env, path::PathBuf};

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::{
    artifact::ImageReference,
    docker::{
        DockerSocket, NetworkSelection,
        models::{
            ContainerInspectResponse, HostConfig, HostConfigCgroupnsModeEnum, MountType,
            NetworkInspect, RestartPolicyNameEnum, Volume, VolumeScopeEnum,
        },
    },
};
use uuid::Uuid;

use super::cli::container_id;

pub(super) const NONCE_LABEL: &str = "rustack.test.server-container.nonce";
pub(super) const ROLE_LABEL: &str = "rustack.test.server-container.role";
pub(super) const ENGINE_LABEL: &str = "rustack.test.server-container.engine";
pub(super) const MEMORY: i64 = 536_870_912;
pub(super) const TEMPORARY: &str = "rw,noexec,nosuid,nodev,size=64m,mode=1777";

#[derive(Debug)]
pub(super) struct Inputs {
    pub program: PathBuf,
    pub client: String,
    pub source_socket: String,
    pub gid: String,
    pub network: String,
    pub image: String,
    pub sdk_image: String,
    pub relay: String,
    pub architecture: String,
}
impl Inputs {
    pub fn load() -> Result<Self> {
        let program = env::var("RUSTACK_DOCKER_BIN")?;
        ensure!(
            program.len() <= 4096
                && PathBuf::from(&program).is_absolute()
                && !program.chars().any(char::is_control),
            "absolute Docker CLI required"
        );
        let client = super::super::process::socket()?;
        let _: DockerSocket = client.parse()?;
        let source_socket = env::var("RUSTACK_SERVER_DAEMON_SOCKET")?;
        socket_source(&source_socket)?;
        let gid = env::var("RUSTACK_SERVER_SOCKET_GID")?;
        socket_gid(&gid)?;
        let network = env::var("RUSTACK_RUNTIME_NETWORK")?;
        let _: NetworkSelection = network.parse()?;
        ensure!(
            network != "none",
            "server container requires an explicit user bridge"
        );
        let image = image_variable("RUSTACK_SERVER_IMAGE")?;
        let sdk_image = image_variable("RUSTACK_HANDLER_SDK_IMAGE")?;
        let relay = image_variable("RUSTACK_RUNTIME_IMAGE")?;
        let architecture = super::super::process::architecture()?;
        Ok(Self {
            program: PathBuf::from(program),
            client,
            source_socket,
            gid,
            network,
            image,
            sdk_image,
            relay,
            architecture,
        })
    }
}

fn image_variable(key: &str) -> Result<String> {
    let value = env::var(key)?;
    let _: ImageReference = value.parse()?;
    Ok(value)
}

pub(super) fn socket_source(value: &str) -> Result<()> {
    ensure!(
        (2..=256).contains(&value.len())
            && value.starts_with('/')
            && value
                .get(1..)
                .context("socket path suffix")?
                .split('/')
                .all(|part| !part.is_empty()
                    && part != "."
                    && part != ".."
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))),
        "invalid explicit daemon-side socket path"
    );
    Ok(())
}

pub(super) fn socket_gid(value: &str) -> Result<u32> {
    ensure!(
        (1..=10).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit()),
        "invalid explicit daemon socket GID grammar"
    );
    let gid: u32 = value.parse()?;
    ensure!(
        gid <= 2_147_483_647 && gid.to_string() == value,
        "invalid daemon socket GID range"
    );
    Ok(gid)
}

#[derive(Debug)]
pub(super) struct Expected {
    pub name: String,
    pub volume: String,
    pub nonce: String,
    pub engine: Uuid,
    pub source: String,
    pub network: String,
    pub socket: String,
    pub gid: String,
}
impl Expected {
    pub fn labels(&self, role: &str) -> BTreeMap<String, String> {
        BTreeMap::from([
            (NONCE_LABEL.into(), self.nonce.clone()),
            (ROLE_LABEL.into(), role.into()),
            (ENGINE_LABEL.into(), self.engine.to_string()),
        ])
    }
}

pub(super) fn source_id(value: &str) -> Result<String> {
    let digest = value
        .strip_prefix("sha256:")
        .context("source image ID prefix")?;
    container_id(digest)?;
    Ok(value.to_owned())
}

pub(super) fn network(network: &NetworkInspect, requested: &str) -> Result<String> {
    let id = network.id.as_deref().context("network full ID")?;
    container_id(id)?;
    let name = network.name.as_deref().context("network name")?;
    ensure!(
        (requested == id || requested == name)
            && !matches!(name, "host" | "bridge" | "none" | "default")
            && network.driver.as_deref() == Some("bridge")
            && network.scope.as_deref() == Some("local")
            && network.ingress == Some(false)
            && network.config_only == Some(false)
            && network
                .options
                .as_ref()
                .and_then(|o| o.get("com.docker.network.bridge.default_bridge"))
                .is_none_or(|value| value == "false"),
        "server fixture network differs"
    );
    Ok(id.to_owned())
}

pub(super) fn volume(volume: &Volume, expected: &Expected) -> Result<()> {
    ensure!(
        volume.name == expected.volume
            && volume.driver == "local"
            && expected
                .labels("server-data")
                .iter()
                .all(|(key, value)| volume.labels.get(key) == Some(value))
            && volume.options.is_empty()
            && volume.scope == Some(VolumeScopeEnum::LOCAL)
            && volume.cluster_volume.is_none()
            && volume.mountpoint.starts_with('/')
            && volume.mountpoint.len() <= 4096,
        "server volume ownership differs"
    );
    Ok(())
}

pub(super) fn server(
    view: &ContainerInspectResponse,
    expected: &Expected,
    data: &Volume,
) -> Result<()> {
    volume(data, expected)?;
    container_id(view.id.as_deref().context("server container ID")?)?;
    let config = view.config.as_ref().context("server configuration")?;
    let labels = config.labels.as_ref().context("server labels")?;
    ensure!(
        view.name.as_deref() == Some(format!("/{}", expected.name).as_str())
            && view.image.as_deref() == Some(expected.source.as_str())
            && config.image.as_deref() == Some(expected.source.as_str())
            && config.user.as_deref() == Some("65532:65532")
            && config.working_dir.as_deref() == Some("/data")
            && config.entrypoint.as_deref() == Some(["/rustack".to_owned()].as_slice())
            && config.cmd.as_ref().is_none_or(Vec::is_empty)
            && expected
                .labels("server")
                .iter()
                .all(|(key, value)| labels.get(key) == Some(value)),
        "server immutable identity or execution policy differs"
    );
    let host = view.host_config.as_ref().context("server host policy")?;
    host_policy(host, expected)?;
    mount_policy(view, host, expected, data)?;
    Ok(())
}

fn host_policy(host: &HostConfig, expected: &Expected) -> Result<()> {
    ensure!(
        host.readonly_rootfs == Some(true)
            && host.privileged == Some(false)
            && host.auto_remove == Some(false)
            && host.publish_all_ports == Some(false)
            && host.memory == Some(MEMORY)
            && host.memory_swap == Some(MEMORY)
            && host.nano_cpus == Some(1_000_000_000)
            && host.pids_limit == Some(128)
            && host.oom_kill_disable != Some(true)
            && host.cap_drop.as_deref() == Some(["ALL".to_owned()].as_slice())
            && host.cap_add.as_ref().is_none_or(Vec::is_empty)
            && host.security_opt.as_deref()
                == Some(["no-new-privileges:true".to_owned()].as_slice())
            && host.group_add.as_deref() == Some([expected.gid.clone()].as_slice())
            && host.network_mode.as_deref() == Some(expected.network.as_str())
            && host.pid_mode.as_deref().is_none_or(str::is_empty)
            && host.cgroupns_mode == Some(HostConfigCgroupnsModeEnum::PRIVATE)
            && host.devices.as_ref().is_none_or(Vec::is_empty)
            && host.device_requests.as_ref().is_none_or(Vec::is_empty)
            && host.volumes_from.as_ref().is_none_or(Vec::is_empty)
            && host.binds.as_ref().is_none_or(Vec::is_empty),
        "server kernel/security policy differs"
    );
    let restart = host
        .restart_policy
        .as_ref()
        .context("server restart policy")?;
    ensure!(
        restart.name == Some(RestartPolicyNameEnum::NO) && restart.maximum_retry_count == Some(0),
        "server restart policy differs"
    );
    let logs = host.log_config.as_ref().context("server log policy")?;
    let options = logs.config.as_ref().context("server bounded log options")?;
    ensure!(
        logs.typ.as_deref() == Some("json-file")
            && options.len() == 2
            && options.get("max-size").map(String::as_str) == Some("1m")
            && options.get("max-file").map(String::as_str) == Some("1"),
        "server log bounds differ"
    );
    let temporary = host.tmpfs.as_ref().context("server temporary mount")?;
    ensure!(
        temporary.len() == 1 && temporary.get("/tmp").map(String::as_str) == Some(TEMPORARY),
        "server temporary storage policy differs"
    );
    let bindings = host
        .port_bindings
        .as_ref()
        .context("server published port request")?;
    ensure!(
        bindings.len() == 1
            && bindings
                .get("4566/tcp")
                .and_then(Option::as_ref)
                .is_some_and(|rows| rows.len() == 1
                    && rows
                        .first()
                        .is_some_and(|row| row.host_ip.as_deref() == Some("127.0.0.1"))),
        "server loopback publication differs"
    );
    Ok(())
}

fn mount_policy(
    view: &ContainerInspectResponse,
    host: &HostConfig,
    expected: &Expected,
    data: &Volume,
) -> Result<()> {
    let requested = host.mounts.as_ref().context("server requested mounts")?;
    ensure!(
        requested.len() == 2
            && requested
                .iter()
                .filter(|m| m.target.as_deref() == Some("/data"))
                .count()
                == 1
            && requested
                .iter()
                .filter(|m| m.target.as_deref() == Some("/run/rustack/docker.sock"))
                .count()
                == 1
            && requested
                .iter()
                .all(|mount| (mount.typ == Some(MountType::BIND)
                    && mount.source.as_deref() == Some(expected.socket.as_str())
                    && mount.target.as_deref() == Some("/run/rustack/docker.sock")
                    && mount.read_only == Some(true))
                    || (mount.typ == Some(MountType::VOLUME)
                        && mount.source.as_deref() == Some(expected.volume.as_str())
                        && mount.target.as_deref() == Some("/data")
                        && mount.read_only != Some(true))),
        "server mount request differs"
    );
    let mounts = view.mounts.as_ref().context("server actual mounts")?;
    ensure!(
        (2..=3).contains(&mounts.len())
            && mounts
                .iter()
                .filter(|m| m.destination.as_deref() == Some("/data"))
                .count()
                == 1
            && mounts
                .iter()
                .filter(|m| m.destination.as_deref() == Some("/run/rustack/docker.sock"))
                .count()
                == 1
            && mounts
                .iter()
                .all(|mount| match mount.destination.as_deref() {
                    Some("/data") =>
                        mount.typ.as_deref() == Some("volume")
                            && mount.name.as_deref() == Some(expected.volume.as_str())
                            && mount.rw == Some(true)
                            && mount.driver.as_deref() == Some("local")
                            && mount.source.as_deref() == Some(data.mountpoint.as_str()),
                    Some("/run/rustack/docker.sock") =>
                        mount.typ.as_deref() == Some("bind")
                            && mount.source.as_deref() == Some(expected.socket.as_str())
                            && mount.rw == Some(false),
                    Some("/tmp") => mount.typ.as_deref() == Some("tmpfs"),
                    _ => false,
                }),
        "server actual mount affiliation differs"
    );
    Ok(())
}

pub(super) fn endpoint(view: &ContainerInspectResponse, expected: &Expected) -> Result<String> {
    let network = view
        .network_settings
        .as_ref()
        .context("server network settings")?;
    let networks = network
        .networks
        .as_ref()
        .context("server attached networks")?;
    ensure!(
        networks.len() == 1
            && networks.values().all(|entry| entry.network_id.as_deref()
                == Some(expected.network.as_str())
                && entry
                    .aliases
                    .as_ref()
                    .is_some_and(|names| names.contains(&expected.name))),
        "server DNS/network affiliation differs"
    );
    let ports = network
        .ports
        .as_ref()
        .context("server actual publication")?;
    ensure!(ports.len() == 1, "extra server published port");
    let rows = ports
        .get("4566/tcp")
        .and_then(Option::as_ref)
        .context("server host port")?;
    ensure!(rows.len() == 1, "ambiguous server host publication");
    let row = rows.first().context("empty server port list")?;
    ensure!(
        row.host_ip.as_deref() == Some("127.0.0.1"),
        "server port is not loopback"
    );
    let text = row.host_port.as_deref().context("server assigned port")?;
    let port: u16 = text.parse()?;
    ensure!(
        port > 0 && port.to_string() == text,
        "server assigned port grammar"
    );
    Ok(format!("http://127.0.0.1:{port}"))
}
