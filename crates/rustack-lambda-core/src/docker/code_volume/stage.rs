//! One independently retained stage; no observer can abort accepted Docker IO.

use std::{collections::HashMap, sync::Arc};

use tokio::time::Instant;

use super::entry::{Context, Entry};
use crate::docker::{
    DockerError,
    models::{ContainerCreateBody, HostConfig, Mount, MountType, MountVolumeOptions},
    mutation::MutationGuard,
    ownership::codes::{CodeRecord, LoaderState, VolumeState},
    recovery::code::retire_loader,
};

pub(super) fn start(entry: &Arc<Entry>, context: Arc<Context>) {
    let retained = entry.clone();
    let runtime = context.runtime.clone();
    entry.stage.start_on(&runtime, move || async move {
        let result = prepare(&retained, &context).await;
        retained.outcome.send_replace(Some(result));
        Ok(())
    });
}
fn io(entry: &Entry, context: &Context) -> Instant {
    entry.plan.deadline.min(context.config.deadline())
}

#[tracing::instrument(skip_all)]
async fn prepare(entry: &Entry, context: &Context) -> Result<(), DockerError> {
    entry.check(context)?;
    super::validate_base(&entry.plan.base, &context.docker, io(entry, context)).await?;
    entry.check(context)?;
    let mut record = prepare_volume(entry, context).await?;
    let mutation = MutationGuard::new();
    entry
        .progress
        .receipts
        .send_modify(|receipts| receipts.loader = Some(mutation.receipt()));
    let mut next = record.clone();
    next.loader = LoaderState::Uncertain;
    save(entry, context, &mut record, next).await?;
    entry.check(context)?;
    #[cfg(test)]
    context.fault(9);
    let body = loader_body(entry, &record, context)?;
    let created = context
        .docker
        .create_container_inner(
            &record.loader_key()?,
            &body,
            Some(entry.plan.base.resolved().metadata().architecture),
            io(entry, context),
            Some(mutation),
        )
        .await?;
    let inspected = context
        .docker
        .inspect_container(&created.id.parse()?, io(entry, context))
        .await?;
    let id = record.owned_loader(context.directory.owner_id(), &inspected)?;
    let mut next = record.clone();
    next.loader = LoaderState::Known(id);
    save(entry, context, &mut record, next).await?;
    entry.check(context)?;
    #[cfg(test)]
    context.fault(4);
    let archive = entry
        .plan
        .archive
        .borrow()
        .clone()
        .ok_or(DockerError::Worker)?;
    let uploaded = context
        .docker
        .upload_code(
            &record.loader_key()?,
            &archive,
            entry
                .plan
                .deadline
                .min(Instant::now() + context.config.upload),
        )
        .await;
    drop(archive);
    // The stage awaits the original call through its wire/driver join.
    entry.plan.archive.send_replace(None);
    uploaded?;
    entry.check(context)?;
    if !Box::pin(retire_loader(
        &context.directory,
        &context.docker,
        &mut record,
        io(entry, context),
    ))
    .await?
    {
        return Err(DockerError::Quarantined);
    }
    entry.check(context)?;
    tracing::debug!(environment_id=%record.environment,"code volume prepared; loader retired");
    Ok(())
}
async fn prepare_volume(entry: &Entry, context: &Context) -> Result<CodeRecord, DockerError> {
    let mut record = entry.plan.record.clone();
    context
        .directory
        .prepare_code(record.clone(), io(entry, context))
        .await?;
    entry.check(context)?;
    let mutation = MutationGuard::new();
    entry
        .progress
        .receipts
        .send_modify(|receipts| receipts.volume = Some(mutation.receipt()));
    let mut next = record.clone();
    next.volume = VolumeState::Uncertain;
    save(entry, context, &mut record, next).await?;
    entry.check(context)?;
    #[cfg(test)]
    context.fault(8);
    let volume = context
        .docker
        .create_volume_tracked(
            &record.volume_key()?,
            &record.labels(context.directory.owner_id(), false),
            io(entry, context),
            Some(mutation),
        )
        .await?;
    record.owned_volume(context.directory.owner_id(), &volume)?;
    let mut next = record.clone();
    next.volume = VolumeState::Known;
    save(entry, context, &mut record, next).await?;
    entry.check(context)?;
    #[cfg(test)]
    context.fault(3);
    Ok(record)
}
async fn save(
    entry: &Entry,
    context: &Context,
    record: &mut CodeRecord,
    next: CodeRecord,
) -> Result<(), DockerError> {
    context
        .directory
        .advance_code(record.clone(), next.clone(), io(entry, context))
        .await?;
    *record = next;
    Ok(())
}

fn loader_body(
    entry: &Entry,
    record: &CodeRecord,
    context: &Context,
) -> Result<ContainerCreateBody, DockerError> {
    Ok(ContainerCreateBody {
        image: Some(
            entry
                .plan
                .base
                .resolved()
                .metadata()
                .image_id
                .as_str()
                .into(),
        ),
        user: Some("65534:65534".into()),
        entrypoint: Some(vec!["/never-start-this-loader".into()]),
        cmd: Some(Vec::new()),
        working_dir: Some("/var/task".into()),
        labels: Some(
            record
                .labels(context.directory.owner_id(), true)
                .into_iter()
                .collect::<HashMap<_, _>>(),
        ),
        host_config: Some(HostConfig {
            network_mode: Some("none".into()),
            readonly_rootfs: Some(true),
            memory: Some(128 * 1024 * 1024),
            nano_cpus: Some(1_000_000_000),
            pids_limit: Some(64),
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            mounts: Some(vec![Mount {
                typ: Some(MountType::VOLUME),
                source: Some(record.volume_key()?.as_str().into()),
                target: Some("/var/task".into()),
                read_only: Some(false),
                volume_options: Some(MountVolumeOptions {
                    no_copy: Some(true),
                    ..MountVolumeOptions::default()
                }),
                ..Mount::default()
            }]),
            ..HostConfig::default()
        }),
        ..ContainerCreateBody::default()
    })
}
