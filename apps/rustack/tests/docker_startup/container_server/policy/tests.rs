//! Independent Docker-shaped fixture; mutations must remove cleanup authority.

use serde_json::{Value, json};

use super::{super::cli::decode, *};

fn fixture() -> Result<(Expected, Volume, Value)> {
    let expected = Expected {
        name: "rustack-server-test".into(),
        volume: "rustack-server-data-test".into(),
        nonce: "1".repeat(32),
        engine: Uuid::from_u128(2),
        source: format!("sha256:{}", "a".repeat(64)),
        network: "f".repeat(64),
        socket: "/var/run/docker.sock".into(),
        gid: "991".into(),
    };
    let data: Volume = decode(&serde_json::to_vec(&json!({
        "Name":"rustack-server-data-test","Driver":"local","Mountpoint":"/volumes/test/_data",
        "Scope":"local","Options":null,"Labels":{
            "rustack.test.server-container.nonce":"1".repeat(32),
            "rustack.test.server-container.role":"server-data",
            "rustack.test.server-container.engine":Uuid::from_u128(2).to_string()}}))?)?;
    let view = json!({
        "Id":"c".repeat(64),"Name":"/rustack-server-test","Image":format!("sha256:{}","a".repeat(64)),
        "Config":{"Image":format!("sha256:{}","a".repeat(64)),"User":"65532:65532","WorkingDir":"/data",
            "Entrypoint":["/rustack"],"Cmd":[],"Labels":{
                "rustack.test.server-container.nonce":"1".repeat(32),
                "rustack.test.server-container.role":"server",
                "rustack.test.server-container.engine":Uuid::from_u128(2).to_string()}},
        "HostConfig":{"ReadonlyRootfs":true,"Privileged":false,"AutoRemove":false,"PublishAllPorts":false,
            "Memory":536_870_912,"MemorySwap":536_870_912,"NanoCpus":1_000_000_000,"PidsLimit":128,
            "CapDrop":["ALL"],"SecurityOpt":["no-new-privileges:true"],"GroupAdd":["991"],
            "NetworkMode":"f".repeat(64),"PidMode":"","CgroupnsMode":"private",
            "RestartPolicy":{"Name":"no","MaximumRetryCount":0},
            "LogConfig":{"Type":"json-file","Config":{"max-size":"1m","max-file":"1"}},
            "Tmpfs":{"/tmp":"rw,noexec,nosuid,nodev,size=64m,mode=1777"},
            "Mounts":[{"Type":"bind","Source":"/var/run/docker.sock","Target":"/run/rustack/docker.sock","ReadOnly":true},
                {"Type":"volume","Source":"rustack-server-data-test","Target":"/data"}],
            "PortBindings":{"4566/tcp":[{"HostIp":"127.0.0.1","HostPort":""}]}},
        "Mounts":[{"Type":"bind","Source":"/var/run/docker.sock","Destination":"/run/rustack/docker.sock","RW":false},
            {"Type":"volume","Name":"rustack-server-data-test","Source":"/volumes/test/_data","Destination":"/data","RW":true,"Driver":"local"}],
        "NetworkSettings":{"Networks":{"qualified":{"NetworkID":"f".repeat(64),"Aliases":["rustack-server-test"]}},
            "Ports":{"4566/tcp":[{"HostIp":"127.0.0.1","HostPort":"49153"}]}},
        "State":{"Running":true,"OOMKilled":false,"ExitCode":0,"Dead":false}
    });
    Ok((expected, data, view))
}

#[test]
fn test_should_require_exact_server_identity_security_and_named_mount_affiliation() -> Result<()> {
    let (expected, data, view) = fixture()?;
    let inspected: ContainerInspectResponse = decode(&serde_json::to_vec(&view)?)?;
    server(&inspected, &expected, &data)?;
    for (path, replacement) in [
        ("/Name", json!("/foreign")),
        ("/Image", json!(format!("sha256:{}", "b".repeat(64)))),
        (
            "/Config/Labels/rustack.test.server-container.nonce",
            json!("foreign"),
        ),
        ("/Config/User", json!("0:0")),
        ("/HostConfig/ReadonlyRootfs", json!(false)),
        ("/HostConfig/Privileged", json!(true)),
        ("/HostConfig/NetworkMode", json!("host")),
        ("/HostConfig/MemorySwap", json!(-1)),
        ("/HostConfig/GroupAdd", json!(["0"])),
        (
            "/HostConfig/PortBindings/4566~1tcp/0/HostIp",
            json!("0.0.0.0"),
        ),
        ("/Mounts/0/Source", json!("/foreign/socket")),
        ("/Mounts/1/Name", json!("foreign-volume")),
        ("/Mounts/1/Source", json!("/volumes/foreign/_data")),
        ("/Mounts/1/RW", json!(false)),
    ] {
        let mut changed = view.clone();
        *changed
            .pointer_mut(path)
            .context("policy fixture mutation path")? = replacement;
        let inspected = decode(&serde_json::to_vec(&changed)?)?;
        assert!(
            server(&inspected, &expected, &data).is_err(),
            "accepted policy mutation {path}"
        );
    }
    Ok(())
}

#[test]
fn test_should_refuse_wrong_actual_dns_network_or_public_host_port_metadata() -> Result<()> {
    let (expected, _, view) = fixture()?;
    let inspected = decode(&serde_json::to_vec(&view)?)?;
    assert_eq!(endpoint(&inspected, &expected)?, "http://127.0.0.1:49153");
    for (path, replacement) in [
        (
            "/NetworkSettings/Networks/qualified/NetworkID",
            json!("e".repeat(64)),
        ),
        (
            "/NetworkSettings/Networks/qualified/Aliases",
            json!(["foreign"]),
        ),
        (
            "/NetworkSettings/Ports/4566~1tcp/0/HostIp",
            json!("0.0.0.0"),
        ),
        ("/NetworkSettings/Ports/4566~1tcp/0/HostPort", json!("0")),
        (
            "/NetworkSettings/Ports/4566~1tcp/0/HostPort",
            json!("049153"),
        ),
        (
            "/NetworkSettings/Ports/4566~1tcp/0/HostPort",
            json!("65536"),
        ),
    ] {
        let mut changed = view.clone();
        *changed
            .pointer_mut(path)
            .context("endpoint fixture mutation path")? = replacement;
        let inspected = decode(&serde_json::to_vec(&changed)?)?;
        assert!(
            endpoint(&inspected, &expected).is_err(),
            "accepted endpoint mutation {path}"
        );
    }
    Ok(())
}

#[test]
fn test_should_reject_foreign_or_driver_configured_server_volumes() -> Result<()> {
    let (expected, data, _) = fixture()?;
    volume(&data, &expected)?;
    let mut foreign = data.clone();
    foreign.name = "unrelated".into();
    assert!(volume(&foreign, &expected).is_err());
    let mut wrong_driver = data.clone();
    wrong_driver.driver = "nfs".into();
    assert!(volume(&wrong_driver, &expected).is_err());
    let mut configured = data.clone();
    configured.options.insert("device".into(), "/host".into());
    assert!(volume(&configured, &expected).is_err());
    let mut relabeled = data;
    relabeled.labels.clear();
    assert!(volume(&relabeled, &expected).is_err());
    Ok(())
}
