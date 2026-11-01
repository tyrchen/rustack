//! Fixed-path bounded async reads; process/container retirement owns cancellation.

use std::{env::consts::ARCH, io, os::unix::fs::MetadataExt};

use tokio::{
    fs::{File, metadata},
    io::AsyncReadExt,
    time::{Instant, timeout_at},
};

use super::{ProbeError, ResourceReport, parse};
use crate::protocol::{Architecture, AttemptId};

async fn read(path: &'static str, cap: usize) -> Result<String, ProbeError> {
    let mut bytes = Vec::new();
    File::open(path)
        .await?
        .take(
            u64::try_from(cap)
                .map_err(|_| ProbeError::Observation)?
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > cap {
        return Err(ProbeError::Observation);
    }
    String::from_utf8(bytes).map_err(|_| ProbeError::Observation)
}
pub(super) async fn collect(
    attempt: AttemptId,
    deadline: Instant,
) -> Result<ResourceReport, ProbeError> {
    timeout_at(deadline, async {
        let memory = read("/sys/fs/cgroup/memory.max", 128).await;
        let (memory_max, swap_max, pids_max, cpu_quota, cpu_period, unified) = match memory {
            Ok(memory) => {
                let (quota, period) = parse::cpu(&read("/sys/fs/cgroup/cpu.max", 128).await?)?;
                (
                    parse::limit(&memory)?,
                    parse::limit(&read("/sys/fs/cgroup/memory.swap.max", 128).await?)?,
                    parse::limit(&read("/sys/fs/cgroup/pids.max", 128).await?)?,
                    quota,
                    period,
                    true,
                )
            }
            Err(ProbeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                let memory =
                    parse::limit(&read("/sys/fs/cgroup/memory/memory.limit_in_bytes", 128).await?)?;
                let total = parse::limit(
                    &read("/sys/fs/cgroup/memory/memory.memsw.limit_in_bytes", 128).await?,
                )?;
                let swap = match (memory, total) {
                    (Some(memory), Some(total)) => {
                        Some(total.checked_sub(memory).ok_or(ProbeError::Observation)?)
                    }
                    _ => None,
                };
                (
                    memory,
                    swap,
                    parse::limit(&read("/sys/fs/cgroup/pids/pids.max", 128).await?)?,
                    parse::limit(&read("/sys/fs/cgroup/cpu/cpu.cfs_quota_us", 128).await?)?,
                    parse::number(&read("/sys/fs/cgroup/cpu/cpu.cfs_period_us", 128).await?)?,
                    false,
                )
            }
            Err(error) => return Err(error),
        };
        let status = parse::status(&read("/proc/self/status", 65536).await?)?;
        let cgroup_root = parse::membership(&read("/proc/self/cgroup", 65536).await?, unified)?;
        let mode = metadata("/tmp").await?.mode() & 0o7777;
        let (root_readonly, temporary) =
            parse::mounts(&read("/proc/self/mountinfo", 65536).await?, mode)?;
        let architecture = match ARCH {
            "aarch64" => Architecture::Arm64,
            "x86_64" => Architecture::X86_64,
            _ => return Err(ProbeError::Configuration),
        };
        Ok(ResourceReport::builder()
            .version(1)
            .attempt(attempt)
            .architecture(architecture)
            .cgroup_root(cgroup_root)
            .memory_max(memory_max)
            .swap_max(swap_max)
            .pids_max(pids_max)
            .cpu_quota(cpu_quota)
            .cpu_period(cpu_period)
            .euid(status.uid)
            .egid(status.gid)
            .cap_effective(status.capabilities)
            .no_new_privileges(status.no_new_privileges)
            .seccomp_mode(status.seccomp)
            .root_readonly(root_readonly)
            .temporary(temporary)
            .build())
    })
    .await
    .map_err(|_| ProbeError::Deadline)?
}
