use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use base64::{Engine, engine::general_purpose::URL_SAFE};
use nix::{sys::stat::Mode, unistd::mkfifo};
use proptest::prelude::*;
use rstest::rstest;
use rustack_core::settings::RuntimeBudgets;
use serde_json::{Value, from_slice};
use tempfile::{TempDir, tempdir};
use tokio::{
    fs, spawn,
    sync::{Semaphore, oneshot},
    task::yield_now,
    time::{Instant, sleep, timeout},
};

use super::{
    Control, RegistryAuthError, open_private, parse, private_file, read_owned, spawn_read,
    validate_observation,
};
use crate::{
    artifact::Architecture,
    docker::{
        DirectoryLease, DockerRuntimeSettings, ImageLeaseConfig, ImageLeaseManager, ImagePinScope,
        RegistryAuthFile, RegistryHost, tests::Daemon,
    },
};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn approved() -> BTreeSet<RegistryHost> {
    BTreeSet::from(["registry.example".parse().unwrap()])
}
fn control() -> Control {
    Control {
        deadline: deadline(),
        cancelled: Arc::new(AtomicBool::new(false)),
    }
}
async fn fixture(text: &str) -> (TempDir, RegistryAuthFile) {
    let root = tempdir().unwrap();
    let canonical = fs::canonicalize(root.path()).await.unwrap();
    let path = canonical.join("credentials.yaml");
    fs::write(&path, text).await.unwrap();
    fs::set_permissions(&path, PermissionsExt::from_mode(0o600))
        .await
        .unwrap();
    (root, path.to_str().unwrap().parse().unwrap())
}
#[rstest]
#[case("{}")]
#[case("registry.example: {username: 'user', password: \"password\"}")]
#[case("---\nregistry.example:\n  identityToken: \"token\\u2603\"\n...")]
#[case("'registry.example':\n  username: 'user'\n  password: 'pas''sword'")]
fn test_should_parse_exact_shallow_quoted_credential_schema(#[case] text: &str) {
    assert!(parse(text, &approved(), &control()).is_ok());
}
#[rstest]
#[case("")]
#[case("[]")]
#[case("secret")]
#[case("registry.example: {}")]
#[case("registry.example: {username: 'user'}")]
#[case("registry.example: {password: 'p'}")]
#[case("registry.example: {identityToken: ''}")]
#[case("registry.example: {identityToken: token}")]
#[case("registry.example: {identityToken: true}")]
#[case("registry.example: {identityToken: 123}")]
#[case("registry.example: {identityToken: null}")]
#[case("registry.example: {identityToken: 'a', identityToken: 'b'}")]
#[case("registry.example: {username: 'u', password: 'p', identityToken: 't'}")]
#[case("registry.example: {username: 'u', password: 'p', extra: 'x'}")]
#[case("registry.example: {identityToken: \"a\\nb\"}")]
#[case("registry.example: {identityToken: !secret 'token'}")]
#[case("registry.example: &secret {identityToken: 'token'}")]
#[case("registry.example: {identityToken: &secret 'token'}")]
#[case("registry.example: {identityToken: *secret}")]
#[case("registry.example: {identityToken: {nested: 'token'}}")]
#[case("registry.example: {identityToken: ['token']}")]
#[case("registry.example: {<<: {identityToken: 'token'}}")]
#[case("registry.example: {identityToken: 'a'}\nregistry.example: {identityToken: 'b'}")]
#[case("REGISTRY.EXAMPLE: {identityToken: 'a'}\nregistry.example: {identityToken: 'b'}")]
#[case("other.example: {identityToken: 'token'}")]
#[case("{}\n---\n{}")]
#[case("{}\ntrailing: [")]
fn test_should_reject_unsafe_ambiguous_or_typed_yaml(#[case] text: &str) {
    let error = parse(text, &approved(), &control()).unwrap_err();
    assert!(!format!("{error:?}: {error}").contains("token"));
}
#[test]
fn test_should_bound_decoded_secrets_collections_and_input() {
    for (field, max) in [
        ("username", 256),
        ("password", 4096),
        ("identityToken", 8192),
    ] {
        let text = if field == "identityToken" {
            format!(
                "registry.example: {{identityToken: '{}' }}",
                "t".repeat(max)
            )
        } else {
            format!(
                "registry.example: {{username: '{}', password: '{}'}}",
                if field == "username" {
                    "u".repeat(max)
                } else {
                    "u".into()
                },
                if field == "password" {
                    "p".repeat(max)
                } else {
                    "p".into()
                }
            )
        };
        assert!(parse(&text, &approved(), &control()).is_ok());
        let too_big = text.replacen(&"t".repeat(max), &"t".repeat(max + 1), 1);
        if field == "identityToken" {
            assert!(parse(&too_big, &approved(), &control()).is_err());
        } else {
            let big = format!(
                "registry.example: {{username: '{}', password: '{}'}}",
                if field == "username" {
                    "u".repeat(max + 1)
                } else {
                    "u".into()
                },
                if field == "password" {
                    "p".repeat(max + 1)
                } else {
                    "p".into()
                }
            );
            assert!(parse(&big, &approved(), &control()).is_err());
        }
    }
    let text = format!("{{}}\n#{}", "x".repeat(65_532));
    assert_eq!(text.len(), 65_536);
    assert!(parse(&text, &approved(), &control()).is_ok());
    assert!(parse(&(text + "x"), &approved(), &control()).is_err());
    let hosts: BTreeSet<_> = (0..33)
        .map(|i| format!("r{i}.example").parse().unwrap())
        .collect();
    let mut raw = String::new();
    for i in 0..33 {
        writeln!(&mut raw, "r{i}.example: {{identityToken: 't'}}").unwrap();
    }
    assert!(parse(&raw, &hosts, &control()).is_err());
    let hosts: BTreeSet<_> = (0..8)
        .map(|i| format!("r{i}.example").parse().unwrap())
        .collect();
    let mut raw = String::new();
    for i in 0..8 {
        writeln!(
            &mut raw,
            "r{i}.example: {{identityToken: \"{}xx\"}}",
            "\\L".repeat(2730)
        )
        .unwrap();
    }
    assert!(raw.len() < 65_536);
    assert!(parse(&raw, &hosts, &control()).is_err());
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn test_should_never_panic_or_expose_arbitrary_yaml(text in ".{0,2048}"){
        if let Err(error)=parse(&text,&approved(),&control()){let shown=format!("{error}");prop_assert!(shown.len()<128);}
    }
}
#[tokio::test]
async fn test_should_load_real_private_file_and_reject_modes_links_and_replacements() {
    let (root, file) = fixture("registry.example: {identityToken: 'PRIVATE-TOKEN'}").await;
    let credentials = file
        .load(approved().into_iter().collect(), deadline())
        .await
        .unwrap();
    assert!(!format!("{credentials:?}").contains("PRIVATE"));
    for mode in [0o644, 0o640, 0o660, 0o700, 0o1600, 0o4600] {
        fs::set_permissions(&file.0, PermissionsExt::from_mode(mode))
            .await
            .unwrap();
        assert!(matches!(
            file.load(approved().into_iter().collect(), deadline())
                .await,
            Err(RegistryAuthError::UnsafeFile)
        ));
    }
    fs::set_permissions(&file.0, PermissionsExt::from_mode(0o600))
        .await
        .unwrap();
    let link = root.path().join("hardlink");
    fs::hard_link(&file.0, &link).await.unwrap();
    assert!(matches!(
        file.load(approved().into_iter().collect(), deadline())
            .await,
        Err(RegistryAuthError::UnsafeFile)
    ));
    fs::remove_file(link).await.unwrap();
    let descriptor = open_private(&file.0, &control()).unwrap();
    let original = private_file(&descriptor).unwrap();
    fs::write(&file.0, "{}\n#changed").await.unwrap();
    assert!(matches!(
        validate_observation(
            &descriptor,
            &original,
            usize::try_from(original.st_size).unwrap()
        ),
        Err(RegistryAuthError::UnsafeFile)
    ));
    fs::remove_file(&file.0).await.unwrap();
    fs::write(&file.0, "{}").await.unwrap();
    fs::set_permissions(&file.0, PermissionsExt::from_mode(0o600))
        .await
        .unwrap();
    assert!(matches!(
        private_file(&descriptor),
        Err(RegistryAuthError::UnsafeFile)
    ));
    file.load(approved().into_iter().collect(), deadline())
        .await
        .unwrap();
}
#[tokio::test]
async fn test_should_reject_leaf_ancestor_symlinks_and_nonregular_files_without_hanging() {
    let (root, file) = fixture("{}").await;
    let canonical = fs::canonicalize(root.path()).await.unwrap();
    let symlink = canonical.join("link");
    fs::symlink(&file.0, &symlink).await.unwrap();
    let source: RegistryAuthFile = symlink.to_str().unwrap().parse().unwrap();
    assert!(
        source
            .load(approved().into_iter().collect(), deadline())
            .await
            .is_err()
    );
    let dir = canonical.join("dir");
    fs::create_dir(&dir).await.unwrap();
    let source: RegistryAuthFile = dir.to_str().unwrap().parse().unwrap();
    assert!(
        source
            .load(approved().into_iter().collect(), deadline())
            .await
            .is_err()
    );
    let ancestor = canonical.join("ancestor");
    fs::symlink(&canonical, &ancestor).await.unwrap();
    let source: RegistryAuthFile = ancestor
        .join("credentials.yaml")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        source
            .load(approved().into_iter().collect(), deadline())
            .await
            .is_err()
    );
    let fifo = canonical.join("fifo");
    mkfifo(&fifo, Mode::from_bits_truncate(0o600)).unwrap();
    let source: RegistryAuthFile = fifo.to_str().unwrap().parse().unwrap();
    assert!(
        timeout(
            Duration::from_secs(1),
            source.load(approved().into_iter().collect(), deadline())
        )
        .await
        .unwrap()
        .is_err()
    );
    fs::set_permissions(&canonical, PermissionsExt::from_mode(0o777))
        .await
        .unwrap();
    assert!(
        file.load(approved().into_iter().collect(), deadline())
            .await
            .is_err()
    );
    fs::set_permissions(&canonical, PermissionsExt::from_mode(0o700))
        .await
        .unwrap();
}
#[tokio::test]
async fn test_should_enforce_real_file_size_utf8_and_approved_set_without_fallback() {
    for text in [
        String::new(),
        "x".repeat(65_537),
        "registry.example: {identityToken: token}".into(),
    ] {
        let (_root, file) = fixture(&text).await;
        assert!(
            file.load(approved().into_iter().collect(), deadline())
                .await
                .is_err()
        );
    }
    let (_root, file) = fixture(&format!("{{}}\n#{}", "x".repeat(65_532))).await;
    file.load(approved().into_iter().collect(), deadline())
        .await
        .unwrap();
    fs::write(&file.0, [0xff]).await.unwrap();
    assert!(
        file.load(approved().into_iter().collect(), deadline())
            .await
            .is_err()
    );
    assert!(file.load(Vec::new(), deadline()).await.is_err());
    assert!(
        file.load(vec!["registry.example".parse().unwrap(); 33], deadline())
            .await
            .is_err()
    );
    assert!(
        file.load(approved().into_iter().collect(), Instant::now())
            .await
            .is_err()
    );
    assert!(
        file.load(
            approved().into_iter().collect(),
            Instant::now() + Duration::from_secs(61)
        )
        .await
        .is_err()
    );
    fs::remove_file(&file.0).await.unwrap();
    let error = file
        .load(approved().into_iter().collect(), deadline())
        .await
        .unwrap_err();
    assert!(!format!("{error:?}:{error}").contains("credentials.yaml"));
}
#[tokio::test]
async fn test_should_retain_all_four_worker_permits_after_observers_are_abandoned() {
    static POOL: Semaphore = Semaphore::const_new(4);
    let (_root, file) = fixture("{}").await;
    let mut releases = Vec::new();
    for _ in 0..4 {
        let (release, held) = mpsc::sync_channel(1);
        let (entered, saw) = oneshot::channel();
        let path = file.0.clone();
        let task = spawn(read_owned(
            path,
            approved().into_iter().collect(),
            deadline(),
            &POOL,
            move |_| {
                let _ = entered.send(());
                held.recv().map_err(|_| RegistryAuthError::Worker)
            },
        ));
        saw.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        releases.push(release);
    }
    assert_eq!(POOL.available_permits(), 0);
    let started = Arc::new(AtomicBool::new(false));
    let observed = started.clone();
    assert!(matches!(
        read_owned(
            file.0.clone(),
            approved().into_iter().collect(),
            Instant::now() + Duration::from_millis(25),
            &POOL,
            move |_| {
                started.store(true, Ordering::Release);
                Ok(())
            }
        )
        .await,
        Err(RegistryAuthError::Deadline)
    ));
    assert!(!observed.load(Ordering::Acquire));
    assert_eq!(POOL.available_permits(), 0);
    for release in releases {
        release.send(()).unwrap();
    }
    timeout(Duration::from_secs(2), async {
        while POOL.available_permits() != 4 {
            yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn test_should_charge_ready_results_and_release_failed_workers() {
    static POOL: Semaphore = Semaphore::const_new(1);
    let (_root, file) = fixture("{}").await;
    let task = spawn_read(
        file.0.clone(),
        approved(),
        control(),
        POOL.acquire().await.unwrap(),
        |_| Ok(()),
    );
    timeout(Duration::from_secs(2), async {
        while !task.is_finished() {
            yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(POOL.available_permits(), 0);
    let (_credentials, permit) = task.await.unwrap().unwrap();
    assert_eq!(POOL.available_permits(), 0);
    drop(permit);
    assert_eq!(POOL.available_permits(), 1);
    assert!(matches!(
        read_owned(
            file.0.clone(),
            approved().into_iter().collect(),
            deadline(),
            &POOL,
            |_| panic!("controlled credential worker failure")
        )
        .await,
        Err(RegistryAuthError::Worker)
    ));
    assert_eq!(POOL.available_permits(), 1);
}
#[tokio::test]
async fn test_should_keep_timed_out_local_read_charged_until_worker_observes_cancellation() {
    static POOL: Semaphore = Semaphore::const_new(1);
    let (_root, file) = fixture("{}").await;
    let (release, held) = mpsc::sync_channel(1);
    let (entered, saw) = oneshot::channel();
    let path = file.0.clone();
    let task = spawn(read_owned(
        path,
        approved().into_iter().collect(),
        Instant::now() + Duration::from_secs(1),
        &POOL,
        move |_| {
            let _ = entered.send(());
            held.recv().map_err(|_| RegistryAuthError::Worker)
        },
    ));
    saw.await.unwrap();
    let result = task.await.unwrap();
    assert!(matches!(result, Err(RegistryAuthError::Deadline)));
    assert_eq!(POOL.available_permits(), 0);
    release.send(()).unwrap();
    timeout(Duration::from_secs(2), async {
        while POOL.available_permits() != 1 {
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn test_should_consume_file_loaded_policy_through_bound_image_preparation() {
    let (root, file) =
        fixture("registry.example: {username: 'PRIVATE-USER', password: 'PRIVATE-PASSWORD'}").await;
    let values = BTreeMap::from([
        ("LAMBDA_DOCKER_RELAY_IMAGE", "relay:v1".to_owned()),
        ("LAMBDA_DOCKER_NETWORK", "none".into()),
        ("LAMBDA_DOCKER_PULL_POLICY", "ifMissing".into()),
        ("LAMBDA_DOCKER_REGISTRIES", "registry.example".into()),
        (
            "LAMBDA_DOCKER_REGISTRY_AUTH_FILE",
            file.0.to_str().unwrap().to_owned(),
        ),
    ]);
    let settings = DockerRuntimeSettings::from_lookup(
        |key| values.get(key).cloned(),
        &RuntimeBudgets::default(),
        Duration::from_secs(600),
    )
    .unwrap();
    let policy = settings.load_pull_policy(deadline()).await.unwrap();
    let directory = DirectoryLease::acquire(root.path().into(), deadline())
        .await
        .unwrap();
    let mut daemon = Daemon::new();
    let mut wire = daemon.wire().await;
    let bound = wire
        .port()
        .bind_directory(&directory, deadline())
        .await
        .unwrap();
    let mut images = ImageLeaseManager::start_with_pull(
        bound,
        ImagePinScope::new(directory.clone()),
        ImageLeaseConfig::default(),
        policy,
    )
    .unwrap();
    let port = images.port();
    let reference = "registry.example/fn:v1".parse().unwrap();
    let (result, ()) = tokio::join!(
        port.prepare(&reference, Architecture::Arm64, None, deadline()),
        async {
            daemon.next().await.reply(404, b"{}").await;
            let request = daemon.next().await;
            assert!(request.head.starts_with("POST /v1.53/images/create?"));
            let header = request
                .head
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("X-Registry-Auth"))
                        .map(|(_, value)| value.trim())
                })
                .unwrap();
            let auth: Value = from_slice(&URL_SAFE.decode(header).unwrap()).unwrap();
            assert_eq!(auth["username"], "PRIVATE-USER");
            assert_eq!(auth["password"], "PRIVATE-PASSWORD");
            assert_eq!(auth["serveraddress"], "registry.example");
            request.reply(200, b"{\"status\":\"done\"}\n").await;
            daemon
                .next()
                .await
                .reply(200, b"{\"Id\":\"invalid\"}")
                .await;
        }
    );
    assert!(result.is_err());
    images.shutdown(deadline()).await.unwrap();
    assert_eq!(port.status().pins, 0);
    assert_eq!(directory.image_records(deadline()).await.unwrap(), []);
    wire.shutdown().await.unwrap();
    daemon.close().await;
}

#[tokio::test]
async fn test_should_reject_oversized_approved_hosts_before_worker_admission_or_io() {
    static POOL: Semaphore = Semaphore::const_new(0);
    let approved = (0..33)
        .map(|i| format!("r{i}.example").parse().unwrap())
        .collect();
    let result = timeout(
        Duration::from_millis(250),
        read_owned(
            "/missing/unreadable.yaml".into(),
            approved,
            deadline(),
            &POOL,
            |_| panic!("must reject before worker"),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(RegistryAuthError::Configuration)));
}
