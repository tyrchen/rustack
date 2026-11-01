//! Independent inspected authority for a helper that must never execute.

use std::collections::HashMap;

use anyhow::{Context, Result, ensure};
use rustack_lambda_core::docker::models::{
    ContainerInspectResponse, ContainerStateStatusEnum, HostConfigCgroupnsModeEnum, MountType,
    RestartPolicyNameEnum,
};

use super::super::super::policy::Expected;

pub(super) fn inspect(
    view: &ContainerInspectResponse,
    expected: &Expected,
    id: &str,
    name: &str,
) -> Result<()> {
    let config = view.config.as_ref().context("data helper configuration")?;
    let state = view.state.as_ref().context("data helper state")?;
    let labels = config.labels.as_ref().context("data helper labels")?;
    ensure!(
        view.id.as_deref() == Some(id)
            && view.name.as_deref() == Some(format!("/{name}").as_str())
            && view.image.as_deref() == Some(expected.source.as_str())
            && config.image.as_deref() == Some(expected.source.as_str())
            && config.user.as_deref() == Some("65532:65532")
            && expected
                .labels("data-transfer")
                .iter()
                .all(|(key, value)| labels.get(key) == Some(value))
            && state.status == Some(ContainerStateStatusEnum::CREATED)
            && state.running == Some(false)
            && state.pid == Some(0)
            && state.dead != Some(true),
        "data helper identity or lifecycle differs"
    );
    let host = view
        .host_config
        .as_ref()
        .context("data helper host policy")?;
    ensure!(
        host.readonly_rootfs == Some(true)
            && host.privileged == Some(false)
            && host.auto_remove == Some(false)
            && host.publish_all_ports == Some(false)
            && host.memory == Some(33_554_432)
            && host.memory_swap == Some(33_554_432)
            && host.nano_cpus == Some(100_000_000)
            && host.pids_limit == Some(16)
            && host.cgroupns_mode == Some(HostConfigCgroupnsModeEnum::PRIVATE)
            && host.network_mode.as_deref() == Some("none")
            && host.cap_drop.as_deref() == Some(["ALL".to_owned()].as_slice())
            && host.cap_add.as_ref().is_none_or(Vec::is_empty)
            && host.security_opt.as_deref()
                == Some(["no-new-privileges:true".to_owned()].as_slice())
            && host.group_add.as_ref().is_none_or(Vec::is_empty)
            && host.binds.as_ref().is_none_or(Vec::is_empty)
            && host.volumes_from.as_ref().is_none_or(Vec::is_empty)
            && host.devices.as_ref().is_none_or(Vec::is_empty)
            && host.device_requests.as_ref().is_none_or(Vec::is_empty)
            && host.port_bindings.as_ref().is_none_or(HashMap::is_empty),
        "data helper security policy differs"
    );
    let restart = host
        .restart_policy
        .as_ref()
        .context("data helper restart policy")?;
    ensure!(
        restart.name == Some(RestartPolicyNameEnum::NO)
            && restart.maximum_retry_count.unwrap_or_default() == 0,
        "data helper restart policy differs"
    );
    let health = config
        .healthcheck
        .as_ref()
        .context("data helper health policy")?;
    ensure!(
        health.test.as_deref() == Some(["NONE".to_owned()].as_slice()),
        "data helper healthcheck enabled"
    );
    let requested = host
        .mounts
        .as_deref()
        .context("data helper requested mounts")?;
    let [mount] = requested else {
        anyhow::bail!("extra data helper requested mount");
    };
    ensure!(
        mount.typ == Some(MountType::VOLUME)
            && mount.source.as_deref() == Some(expected.volume.as_str())
            && mount.target.as_deref() == Some("/data")
            && mount.read_only != Some(true)
            && mount
                .volume_options
                .as_ref()
                .is_some_and(|options| options.no_copy == Some(true)),
        "data helper requested volume differs"
    );
    let actual = view
        .mounts
        .as_deref()
        .context("data helper actual mounts")?;
    let [mount] = actual else {
        anyhow::bail!("extra data helper actual mount");
    };
    ensure!(
        mount.typ.as_deref() == Some("volume")
            && mount.name.as_deref() == Some(expected.volume.as_str())
            && mount.destination.as_deref() == Some("/data")
            && mount.rw == Some(true)
            && mount.driver.as_deref() == Some("local"),
        "data helper actual volume differs"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::*;
    use crate::container_server::cli::decode;

    fn fixture() -> (Expected, Value) {
        let expected = Expected {
            name: "rustack-server-parent".into(),
            volume: "rustack-server-data-parent".into(),
            nonce: "1".repeat(32),
            engine: Uuid::from_u128(2),
            source: format!("sha256:{}", "a".repeat(64)),
            network: "f".repeat(64),
            socket: "/run/user/501/docker.sock".into(),
            gid: "991".into(),
        };
        let value = json!({
            "Id":"c".repeat(64),"Name":"/rustack-transfer-test","Image":expected.source,
            "State":{"Status":"created","Running":false,"Pid":0,"Dead":false},
            "Config":{"Image":expected.source,"User":"65532:65532","Labels":expected.labels("data-transfer"),
                "Healthcheck":{"Test":["NONE"]}},
            "HostConfig":{"ReadonlyRootfs":true,"Privileged":false,"AutoRemove":false,"PublishAllPorts":false,
                "Memory":33_554_432,"MemorySwap":33_554_432,"NanoCpus":100_000_000,"PidsLimit":16,
                "CgroupnsMode":"private","NetworkMode":"none","CapDrop":["ALL"],
                "CapAdd":[],"Binds":[],"PortBindings":{},
                "SecurityOpt":["no-new-privileges:true"],"RestartPolicy":{"Name":"no","MaximumRetryCount":0},
                "Mounts":[{"Type":"volume","Source":expected.volume,"Target":"/data","ReadOnly":false,
                    "VolumeOptions":{"NoCopy":true}}]},
            "Mounts":[{"Type":"volume","Name":expected.volume,"Destination":"/data","RW":true,"Driver":"local"}]
        });
        (expected, value)
    }

    #[test]
    fn test_should_accept_only_an_owned_never_started_single_volume_helper() -> Result<()> {
        let (expected, value) = fixture();
        let view = decode(&serde_json::to_vec(&value)?)?;
        inspect(&view, &expected, &"c".repeat(64), "rustack-transfer-test")
    }

    #[rstest::rstest]
    #[case("/Id", json!("d".repeat(64)))]
    #[case("/Image", json!(format!("sha256:{}", "b".repeat(64))))]
    #[case("/Name", json!("/foreign"))]
    #[case("/Config/User", json!("0:0"))]
    #[case("/Config/Labels", json!({}))]
    #[case("/State/Status", json!("exited"))]
    #[case("/State/Running", json!(true))]
    #[case("/State/Pid", json!(2))]
    #[case("/HostConfig/NetworkMode", json!("host"))]
    #[case("/HostConfig/Privileged", json!(true))]
    #[case("/HostConfig/CapAdd", json!(["SYS_ADMIN"]))]
    #[case("/HostConfig/Binds", json!(["/run/user/501/docker.sock:/socket"]))]
    #[case("/HostConfig/PortBindings", json!({"80/tcp":[{"HostPort":"80"}]}))]
    #[case("/HostConfig/Mounts/0/ReadOnly", json!(true))]
    #[case("/HostConfig/Mounts/0/VolumeOptions/NoCopy", json!(false))]
    #[case("/Mounts/0/Name", json!("foreign-volume"))]
    #[case("/Mounts/0/RW", json!(false))]
    #[case("/Config/Healthcheck/Test", json!(["CMD","/rustack"]))]
    fn test_should_reject_identity_execution_privilege_and_mount_substitution(
        #[case] pointer: &str,
        #[case] replacement: Value,
    ) -> Result<()> {
        let (expected, mut value) = fixture();
        *value
            .pointer_mut(pointer)
            .context("controlled fixture pointer")? = replacement;
        let view = decode(&serde_json::to_vec(&value)?)?;
        assert!(inspect(&view, &expected, &"c".repeat(64), "rustack-transfer-test").is_err());
        Ok(())
    }
}
