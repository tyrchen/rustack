use std::{env, time::Duration};

use anyhow::{Context, Ok as anyhow_ok, Result, ensure};
use tokio::{fs::read, time::Instant};

use crate::docker::{DirectoryLease, DockerError, DockerWire};

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

#[tokio::test]
#[ignore = "requires two explicit distinct Docker daemons"]
async fn test_should_bind_real_installation_across_reacquisition_and_reject_other_daemon()
-> Result<()> {
    let socket =
        env::var("RUSTACK_RUNTIME_DOCKER_SOCKET").context("explicit primary Docker socket")?;
    let other = env::var("RUSTACK_OTHER_DOCKER_SOCKET").context("explicit other Docker socket")?;
    let root = tempfile::tempdir()?;
    let mut wire = DockerWire::connect(format!("unix://{socket}").parse()?, deadline()).await?;
    let mut second = None;
    let exercise = async {
        let directory = DirectoryLease::acquire(root.path().into(), deadline()).await?;
        let bound = wire.port().bind_directory(&directory, deadline()).await?;
        let first_id = bound
            .info(deadline())
            .await?
            .id
            .context("engine identity")?;
        let previous_owner = directory.owner_id();
        let previous_run = directory.run_id();
        drop(bound);
        drop(directory);
        wire.shutdown().await?;
        wire = DockerWire::connect(format!("unix://{socket}").parse()?, deadline()).await?;
        let next = DirectoryLease::acquire(root.path().into(), deadline()).await?;
        ensure!(
            next.owner_id() == previous_owner && next.run_id() != previous_run,
            "directory identities did not follow restart contract"
        );
        let bound = wire.port().bind_directory(&next, deadline()).await?;
        ensure!(
            bound.info(deadline()).await?.id.as_deref() == Some(first_id.as_str()),
            "installation identity changed"
        );
        let path = root.path().join(".rustack-lambda-docker/daemon.v1");
        let before = read(&path).await?;
        second = Some(DockerWire::connect(format!("unix://{other}").parse()?, deadline()).await?);
        let port = second.as_ref().context("second wire")?.port();
        ensure!(
            port.info(deadline()).await?.id.as_deref() != Some(first_id.as_str()),
            "explicit daemons are not distinct installations"
        );
        ensure!(
            matches!(
                port.bind_directory(&next, deadline()).await,
                Err(DockerError::Ownership(_))
            ),
            "different daemon was accepted"
        );
        ensure!(
            read(path).await? == before,
            "failed binding changed authoritative state"
        );
        anyhow_ok(())
    }
    .await;
    let second_cleanup = match second.as_mut() {
        Some(owner) => owner.shutdown().await,
        None => Ok(()),
    };
    let first_cleanup = wire.shutdown().await;
    exercise?;
    second_cleanup?;
    first_cleanup?;
    Ok(())
}
