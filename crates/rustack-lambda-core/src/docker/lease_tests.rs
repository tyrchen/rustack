use std::{
    future::Future,
    task::{Context, Waker},
};

use serde_json::{Value, json};
use tokio::{
    sync::Notify,
    time::{sleep_until, timeout},
};

use super::*;
use crate::docker::{
    DirectoryLease, DirectoryLeaseError, DockerWire,
    tests::{Daemon, IncomingRequest},
};

fn digest(letter: char) -> String {
    format!("sha256:{}", letter.to_string().repeat(64))
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn scope() -> ImagePinScope {
    ImagePinScope::testing(Uuid::new_v4(), Uuid::new_v4()).unwrap()
}
fn config(pins: u16, jobs: u8) -> ImageLeaseConfig {
    ImageLeaseConfig::new(pins, jobs, Duration::from_secs(1)).unwrap()
}
fn metadata(parent: &str, platform: bool) -> Value {
    let child = if parent == digest('a') {
        digest('c')
    } else {
        digest('d')
    };
    json!({"Id":if platform {&child} else {parent},"Os":"linux","Architecture":"arm64","Size":64,
        "Descriptor":{"mediaType":if platform {"application/vnd.oci.image.manifest.v1+json"} else {"application/vnd.oci.image.index.v1+json"},
            "digest":if platform {&child} else {parent},"platform":{"os":"linux","architecture":"arm64"}},
        "Config":{"Entrypoint":["/entry"],"Cmd":[if parent==digest('a') {"old_handler"} else {"new_handler"}],"Env":["SECRET=private_value"],"WorkingDir":"/task","User":"1000"}})
}
fn manifests(parent: &str, zero: bool) -> Value {
    let mut value = metadata(parent, false);
    let child = if parent == digest('a') {
        digest('c')
    } else {
        digest('d')
    };
    value["Manifests"] = json!([{"ID":child,"Descriptor":{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":child,"platform":{"os":"linux","architecture":"arm64"}},
        "Available":true,"Kind":"image","Size":{"Content":64,"Total":99999},
        "ImageData":{"Containers":[],"Platform":{"os":"linux","architecture":"arm64"},"Size":{"Unpacked":if zero {0} else {4096}}}}]);
    value
}

#[derive(Debug, Default)]
pub(crate) struct Stats {
    pub(crate) tags: AtomicUsize,
    pub(crate) deletes: AtomicUsize,
    pub(crate) absent: AtomicUsize,
    pub(crate) aliases: AtomicUsize,
    changed: Notify,
}
impl Stats {
    pub(crate) async fn until(&self, condition: impl Fn() -> bool) {
        timeout(Duration::from_secs(10), async {
            loop {
                let changed = self.changed.notified();
                if condition() {
                    break;
                }
                changed.await;
            }
        })
        .await
        .unwrap();
    }
    fn notify(&self) {
        self.changed.notify_waiters();
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mode {
    Normal,
    Reject400,
    Reject404,
    Lost,
    PostCommit500,
    Late,
    Late500,
    Never,
    Wrong,
    ZeroSize,
    LostDelete,
    HoldDelete,
}
pub(crate) enum Command {
    Retarget,
    Repair,
    Commit,
    AllowDeletes,
}
struct Pending {
    alias: String,
    parent: String,
    at: Option<Instant>,
    request: Option<IncomingRequest>,
}

#[derive(TypedBuilder)]
struct Script {
    daemon: Daemon,
    mode: Mode,
    stats: Arc<Stats>,
    commands: mpsc::Receiver<Command>,
    stop: watch::Receiver<bool>,
    #[builder(default)]
    aliases: HashMap<String, String>,
    #[builder(default)]
    expected: HashMap<String, String>,
    #[builder(default)]
    pending: Option<Pending>,
    #[builder(default)]
    retargeted: bool,
    #[builder(default)]
    allow_deletes: bool,
}
impl Script {
    async fn run(mut self) {
        loop {
            let at = self.pending.as_ref().and_then(|pending| pending.at);
            tokio::select! {
                biased;
                _ = self.stop.changed()=>break,
                () = sleep_until(at.unwrap_or_else(Instant::now)),if at.is_some()=>self.commit(),
                command=self.commands.recv()=>match command {Some(Command::Retarget)=>self.retargeted=true,Some(Command::Repair)=>self.aliases=self.expected.clone(),Some(Command::Commit)=>self.commit(),Some(Command::AllowDeletes)=>self.allow_deletes=true,None=>break},
                request=self.daemon.requests.recv()=>match request {Some(request)=>self.request(request).await,None=>break},
            }
            self.stats
                .aliases
                .store(self.aliases.len(), Ordering::Release);
            self.stats.notify();
        }
        assert_eq!(self.aliases.len(), 0, "fake daemon retains image aliases");
        assert!(self.pending.is_none());
        self.daemon.close().await;
    }
    fn commit(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.aliases.insert(pending.alias, pending.parent);
            drop(pending.request);
        }
    }
    async fn tag(&mut self, request: IncomingRequest, image: &str, query: &str) {
        assert!(image.ends_with("/tag"));
        let parent = image.strip_suffix("/tag").unwrap().to_owned();
        let fields: HashMap<_, _> = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let alias = format!(
            "{}:{}",
            fields.get("repo").unwrap(),
            fields.get("tag").unwrap()
        );
        assert!(alias.starts_with("rustack-internal/lambda-"));
        self.stats.tags.fetch_add(1, Ordering::AcqRel);
        self.expected.insert(alias.clone(), parent.clone());
        match self.mode {
            Mode::Reject400 => request.reply(400, b"{}").await,
            Mode::Reject404 => request.reply(404, b"{}").await,
            Mode::Late | Mode::Never => {
                assert!(self.pending.is_none());
                self.pending = Some(Pending {
                    alias,
                    parent,
                    at: matches!(self.mode, Mode::Late)
                        .then(|| Instant::now() + Duration::from_secs(2)),
                    request: Some(request),
                });
            }
            Mode::Late500 => {
                assert!(self.pending.is_none());
                self.pending = Some(Pending {
                    alias,
                    parent,
                    at: Some(Instant::now() + Duration::from_secs(2)),
                    request: None,
                });
                request.reply(500, b"{}").await;
            }
            _ => {
                self.aliases.insert(
                    alias,
                    if matches!(self.mode, Mode::Wrong) {
                        digest('b')
                    } else {
                        parent
                    },
                );
                match self.mode {
                    Mode::Lost => drop(request),
                    Mode::PostCommit500 => request.reply(500, b"{}").await,
                    _ => request.reply(201, b"").await,
                }
            }
        }
    }

    async fn request(&mut self, request: IncomingRequest) {
        let line = request.head.lines().next().unwrap().to_owned();
        let fields: Vec<_> = line.split_whitespace().collect();
        let method = fields.first().unwrap();
        let target = fields.get(1).unwrap();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let path = percent_encoding::percent_decode_str(path)
            .decode_utf8()
            .unwrap()
            .into_owned();
        let image = path.strip_prefix("/v1.53/images/").unwrap();
        if *method == "POST" {
            self.tag(request, image, query).await;
        } else if *method == "DELETE" {
            assert_eq!(query, "force=false&noprune=true");
            assert!(image.starts_with("rustack-internal/lambda-"));
            self.stats.deletes.fetch_add(1, Ordering::AcqRel);
            if matches!(self.mode, Mode::HoldDelete) && !self.allow_deletes {
                request.reply(500, b"{}").await;
                return;
            }
            let removed = self.aliases.remove(image);
            assert!(removed.is_some());
            if matches!(self.mode, Mode::LostDelete) {
                drop(request);
                return;
            }
            request
                .reply(
                    200,
                    &serde_json::to_vec(&json!([{"Untagged":image}])).unwrap(),
                )
                .await;
        } else {
            assert_eq!(*method, "GET");
            let reference = image.strip_suffix("/json").unwrap();
            let parent = if reference == "worker:v1" {
                Some(if self.retargeted {
                    digest('b')
                } else {
                    digest('a')
                })
            } else if let Some(id) = self.aliases.get(reference) {
                Some(id.clone())
            } else if reference == digest('a') || reference == digest('b') {
                let current = if self.retargeted {
                    digest('b')
                } else {
                    digest('a')
                };
                (current == reference || self.aliases.values().any(|id| id == reference))
                    .then(|| reference.to_owned())
            } else {
                None
            };
            if let Some(parent) = parent {
                let value = if query == "manifests=true" {
                    manifests(&parent, matches!(self.mode, Mode::ZeroSize))
                } else {
                    metadata(&parent, !query.is_empty())
                };
                request
                    .reply(200, &serde_json::to_vec(&value).unwrap())
                    .await;
            } else {
                self.stats.absent.fetch_add(1, Ordering::AcqRel);
                request.reply(404, b"{}").await;
            }
        }
    }
}

pub(crate) struct FakePins {
    pub(crate) wire: DockerWire,
    stop: watch::Sender<bool>,
    pub(crate) commands: mpsc::Sender<Command>,
    task: Option<JoinHandle<()>>,
    pub(crate) stats: Arc<Stats>,
}
impl FakePins {
    pub(crate) async fn new(mode: Mode) -> Self {
        let daemon = Daemon::new();
        let wire = daemon.wire().await;
        let (stop, stopping) = watch::channel(false);
        let (commands, receiver) = mpsc::channel(8);
        let stats = Arc::new(Stats::default());
        let script = Script::builder()
            .daemon(daemon)
            .mode(mode)
            .stats(stats.clone())
            .commands(receiver)
            .stop(stopping)
            .build();
        Self {
            wire,
            stop,
            commands,
            task: Some(tokio::spawn(script.run())),
            stats,
        }
    }
    pub(crate) fn owner(&self, pins: u16, jobs: u8) -> ImageLeaseManager {
        ImageLeaseManager::start(self.wire.port(), scope(), config(pins, jobs)).unwrap()
    }
    pub(crate) async fn close(mut self) {
        self.wire.shutdown().await.unwrap();
        self.stop.send_replace(true);
        self.task.take().unwrap().await.unwrap();
    }
}
impl Drop for FakePins {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

#[tokio::test]
async fn test_should_retain_the_directory_lock_through_abandoned_image_owner_cleanup() {
    let fake = FakePins::new(Mode::HoldDelete).await;
    let directory = tempfile::tempdir().unwrap();
    let lease = DirectoryLease::acquire(directory.path().into(), deadline())
        .await
        .unwrap();
    let identity = lease.owner_id();
    let owner = ImageLeaseManager::start(fake.wire.port(), ImagePinScope::new(lease), config(2, 1))
        .unwrap();
    let image = owner
        .port()
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    drop(image);
    fake.stats
        .until(|| owner.port().status().quarantined > 0)
        .await;
    drop(owner);
    assert!(matches!(
        DirectoryLease::acquire(directory.path().into(), deadline()).await,
        Err(DirectoryLeaseError::InUse)
    ));
    fake.commands.send(Command::AllowDeletes).await.unwrap();
    timeout(Duration::from_secs(10), async {
        loop {
            match DirectoryLease::acquire(directory.path().into(), deadline()).await {
                Ok(lease) => {
                    assert_eq!(lease.owner_id(), identity);
                    break;
                }
                Err(DirectoryLeaseError::InUse) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                other => panic!("unexpected lease result: {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(fake.stats.aliases.load(Ordering::Acquire), 0);
    fake.close().await;
}

#[tokio::test]
async fn test_should_pin_old_code_across_tag_retargeting_and_keep_cloned_artifacts_live() {
    let fake = FakePins::new(Mode::Normal).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    let image = port
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    assert!(image.is_live());
    assert_eq!(image.unpacked_size().get(), 4096);
    assert_eq!(port.status().pins, 1);
    assert!(!format!("{image:?}").contains("private_value"));
    let mut observer = Box::pin(port.wait_empty(deadline()));
    let mut context = Context::from_waker(Waker::noop());
    assert!(observer.as_mut().poll(&mut context).is_pending());
    assert_eq!(port.status().pins, 1);
    fake.commands.send(Command::Retarget).await.unwrap();
    let newer = port
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    drop(observer);
    assert_eq!(image.resolved().metadata().image_id.as_str(), digest('a'));
    assert_eq!(newer.resolved().metadata().image_id.as_str(), digest('b'));
    assert_eq!(
        image
            .resolved()
            .metadata()
            .execution
            .plan(None)
            .unwrap()
            .command(),
        ["old_handler"]
    );
    let captured = image.clone();
    drop(image);
    drop(newer);
    assert!(matches!(
        port.wait_empty(Instant::now() + Duration::from_millis(50))
            .await,
        Err(DockerError::Deadline)
    ));
    assert!(captured.is_live());
    drop(captured);
    port.wait_empty(deadline()).await.unwrap();
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 2);
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 2);
    fake.close().await;
}

#[rstest::rstest]
#[case(Mode::Reject400)]
#[case(Mode::Reject404)]
#[case(Mode::Lost)]
#[case(Mode::PostCommit500)]
#[case(Mode::ZeroSize)]
#[tokio::test]
async fn test_should_reconcile_rejected_and_lost_preparation_outcomes(#[case] mode: Mode) {
    let fake = FakePins::new(mode).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    assert!(
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline()
        )
        .await
        .is_err()
    );
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(port.status().quarantined, 0);
    assert!(port.status().accepting);
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 1);
    assert_eq!(
        fake.stats.deletes.load(Ordering::Acquire),
        usize::from(!matches!(mode, Mode::Reject400 | Mode::Reject404))
    );
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_retain_uncertain_tag_through_cancelled_waiter_and_initial_absence() {
    let fake = FakePins::new(Mode::Late).await;
    let mut owner = fake.owner(1, 1);
    let port = owner.port();
    let call = port.clone();
    let caller = tokio::spawn(async move {
        call.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
    });
    fake.stats
        .until(|| fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    fake.stats
        .until(|| fake.stats.absent.load(Ordering::Acquire) > 0)
        .await;
    assert_eq!(port.status().pins, 1);
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 0);
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 1);
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_bound_preparation_and_pin_admission_and_retain_shutdown_on_timeout() {
    let fake = FakePins::new(Mode::Never).await;
    let mut owner = fake.owner(1, 1);
    let port = owner.port();
    let call = port.clone();
    let caller = tokio::spawn(async move {
        call.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
    });
    fake.stats
        .until(|| fake.stats.tags.load(Ordering::Acquire) == 1)
        .await;
    assert!(matches!(
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline()
        )
        .await,
        Err(DockerError::Busy)
    ));
    assert!(matches!(
        owner
            .shutdown(Instant::now() + Duration::from_millis(50))
            .await,
        Err(DockerError::Deadline)
    ));
    assert!(owner.task.is_some());
    assert_eq!(port.status().pins, 1);
    fake.commands.send(Command::Commit).await.unwrap();
    assert!(caller.await.unwrap().is_err());
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(port.status().pins, 0);
    fake.close().await;
}

#[tokio::test]
async fn test_should_quarantine_observed_alias_conflicts_until_exclusive_owner_repairs_them() {
    let fake = FakePins::new(Mode::Wrong).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    assert!(
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline()
        )
        .await
        .is_err()
    );
    fake.stats.until(|| port.status().quarantined > 0).await;
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 0);
    assert!(matches!(
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline()
        )
        .await,
        Err(DockerError::Quarantined)
    ));
    fake.commands.send(Command::Repair).await.unwrap();
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(port.status().quarantined, 0);
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_join_after_a_shutdown_waiter_is_dropped_and_cache_root_failure() {
    let fake = FakePins::new(Mode::Normal).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    let image = port
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    let mut shutdown = Box::pin(owner.shutdown(deadline()));
    let mut context = Context::from_waker(Waker::noop());
    assert!(shutdown.as_mut().poll(&mut context).is_pending());
    drop(shutdown);
    owner.shutdown(deadline()).await.unwrap();
    assert!(!image.is_live());
    drop(image);
    fake.close().await;

    let fake = FakePins::new(Mode::Normal).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    let image = port
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    owner.task.as_ref().unwrap().abort();
    assert!(matches!(
        owner.shutdown(deadline()).await,
        Err(DockerError::Worker)
    ));
    assert!(!image.is_live());
    assert!(matches!(
        owner.shutdown(deadline()).await,
        Err(DockerError::Worker)
    ));
    assert_eq!(port.status().pins, 0);
    drop(image);
    fake.close().await;
}

#[test]
fn test_should_reject_invalid_pin_scope_and_preparation_configuration() {
    assert!(ImagePinScope::testing(Uuid::nil(), Uuid::new_v4()).is_err());
    let pin = scope().allocate().unwrap();
    assert!(
        pin.reference()
            .as_str()
            .starts_with("rustack-internal/lambda-")
    );
    for (pins, jobs, io) in [
        (0, 1, Duration::from_secs(1)),
        (8193, 1, Duration::from_secs(1)),
        (1, 2, Duration::from_secs(1)),
        (1, 1, Duration::from_nanos(1)),
        (1, 1, Duration::from_secs(61)),
    ] {
        assert!(ImageLeaseConfig::new(pins, jobs, io).is_err());
    }
}

#[tokio::test]
async fn test_should_retire_an_image_sent_to_a_response_channel_with_no_consuming_waiter() {
    let fake = FakePins::new(Mode::Normal).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    let reference = "worker:v1".parse().unwrap();
    let mut request = Box::pin(port.prepare(&reference, Architecture::Arm64, None, deadline()));
    let mut context = Context::from_waker(Waker::noop());
    assert!(request.as_mut().poll(&mut context).is_pending());
    let accepted = timeout_at(deadline(), port.slots.clone().acquire_owned())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(port.status().pins, 1);
    drop(request);
    drop(accepted);
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 1);
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_report_root_failure_even_when_no_pin_has_started() {
    let fake = FakePins::new(Mode::Normal).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    owner.task.as_ref().unwrap().abort();
    timeout_at(deadline(), async {
        while !owner.task.as_ref().unwrap().is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        port.wait_empty(deadline()).await,
        Err(DockerError::Worker)
    ));
    assert!(matches!(
        owner.shutdown(deadline()).await,
        Err(DockerError::Worker)
    ));
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 0);
    fake.close().await;
}

#[rstest::rstest]
#[case(false)]
#[case(true)]
#[tokio::test]
async fn test_should_recover_owned_aliases_after_worker_or_off_runtime_root_failure(
    #[case] off_runtime: bool,
) {
    let fake = FakePins::new(Mode::Normal).await;
    let config = config(1, 1);
    let pin = scope().allocate().unwrap();
    let resolved = ImageResolver::new(fake.wire.port())
        .resolve_local(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    fake.wire
        .port()
        .tag_image_pin(&resolved.metadata().image_id, &pin, deadline())
        .await
        .unwrap();
    let budget = Arc::new(PinBudget::new(1));
    let permit = budget.acquire().unwrap();
    let (control, stop) = watch::channel(false);
    let (release, released) = watch::channel(());
    let (phase, _) = watch::channel(Phase::Known(resolved.metadata().image_id.clone()));
    let active = Arc::new(AtomicBool::new(true));
    let progress = Arc::new(Progress {
        phase,
        released,
        active: active.clone(),
    });
    let image = PinnedImage::new(resolved, 4096.try_into().unwrap(), release, active, stop);
    let (sender, inbox) = mpsc::channel(32);
    let health = Arc::new(Health::default());
    let emergency = Arc::new(CleanupTask::default());
    let id = pin.id;
    let mut jobs: JoinSet<PinJob> = JoinSet::new();
    let job = jobs.spawn(async { panic!("controlled image worker failure") });
    let actor = Actor::builder()
        .docker(fake.wire.port())
        .config(config)
        .inbox(inbox)
        .control(control)
        .health(health.clone())
        .runtime(Handle::current())
        .emergency(emergency.clone())
        .jobs(jobs)
        .records(HashMap::from([(
            id,
            Record {
                pin,
                progress,
                permit,
            },
        )]))
        .index(HashMap::from([(job.id(), id)]))
        .build();
    if off_runtime {
        let thread = std::thread::spawn(move || drop(actor));
        timeout_at(
            deadline(),
            tokio::task::spawn_blocking(move || thread.join()),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        timeout_at(deadline(), emergency.wait())
            .await
            .unwrap()
            .unwrap();
    } else {
        let result = timeout_at(deadline(), tokio::spawn(actor.run()))
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(DockerError::Worker)));
    }
    assert!(health.failed.load(Ordering::Acquire));
    assert!(!image.is_live());
    assert_eq!(budget.count.load(Ordering::Acquire), 0);
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 1);
    emergency.wait().await.unwrap();
    drop(sender);
    drop(image);
    fake.close().await;
}

#[tokio::test]
async fn test_should_retain_a_late_nested_mutation_after_a_complete_http_500() {
    let fake = FakePins::new(Mode::Late500).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    assert!(
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline()
        )
        .await
        .is_err()
    );
    fake.stats
        .until(|| fake.stats.absent.load(Ordering::Acquire) > 0)
        .await;
    assert_eq!(port.status().pins, 1);
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 0);
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 1);
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 1);
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_confirm_absence_after_an_acknowledged_tag_and_lost_delete_response() {
    let fake = FakePins::new(Mode::LostDelete).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    let image = port
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    drop(image);
    port.wait_empty(deadline()).await.unwrap();
    assert_eq!(fake.stats.deletes.load(Ordering::Acquire), 1);
    assert_eq!(port.status().quarantined, 0);
    owner.shutdown(deadline()).await.unwrap();
    fake.close().await;
}

#[tokio::test]
async fn test_should_retain_cleanup_capacity_and_shutdown_join_until_delete_recovers() {
    let fake = FakePins::new(Mode::HoldDelete).await;
    let mut owner = fake.owner(2, 1);
    let port = owner.port();
    let image = port
        .prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline(),
        )
        .await
        .unwrap();
    drop(image);
    fake.stats.until(|| port.status().quarantined > 0).await;
    assert_eq!(port.status().pins, 1);
    assert!(matches!(
        port.prepare(
            &"worker:v1".parse().unwrap(),
            Architecture::Arm64,
            None,
            deadline()
        )
        .await,
        Err(DockerError::Quarantined)
    ));
    assert!(matches!(
        owner
            .shutdown(Instant::now() + Duration::from_millis(50))
            .await,
        Err(DockerError::Deadline)
    ));
    assert!(owner.task.is_some());
    fake.commands.send(Command::AllowDeletes).await.unwrap();
    owner.shutdown(deadline()).await.unwrap();
    assert_eq!(port.status().pins, 0);
    assert_eq!(port.status().quarantined, 0);
    assert_eq!(fake.stats.tags.load(Ordering::Acquire), 1);
    assert!(fake.stats.deletes.load(Ordering::Acquire) >= 2);
    fake.close().await;
}
