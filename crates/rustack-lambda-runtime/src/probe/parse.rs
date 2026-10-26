//! Checked parsers over fixed, bounded kernel text; no paths are derived from it.

use super::{ProbeError, TemporaryReport};

pub(super) fn number(value: &str) -> Result<u64, ProbeError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ProbeError::Observation);
    }
    value.parse().map_err(|_| ProbeError::Observation)
}
pub(super) fn limit(value: &str) -> Result<Option<u64>, ProbeError> {
    if matches!(value.trim(), "max" | "-1") {
        Ok(None)
    } else {
        number(value).map(Some)
    }
}
pub(super) fn cpu(value: &str) -> Result<(Option<u64>, u64), ProbeError> {
    let mut fields = value.split_ascii_whitespace();
    let quota = limit(fields.next().ok_or(ProbeError::Observation)?)?;
    let period = number(fields.next().ok_or(ProbeError::Observation)?)?;
    if fields.next().is_some() {
        return Err(ProbeError::Observation);
    }
    Ok((quota, period))
}
pub(super) struct Status {
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) capabilities: u64,
    pub(super) no_new_privileges: bool,
    pub(super) seccomp: u8,
}
pub(super) fn status(text: &str) -> Result<Status, ProbeError> {
    bounded(text)?;
    let field = |key: &str| -> Result<&str, ProbeError> {
        let mut matching = text
            .lines()
            .filter_map(|line| line.split_once(':'))
            .filter(|(name, _)| *name == key)
            .map(|(_, value)| value.trim());
        let value = matching.next().ok_or(ProbeError::Observation)?;
        if matching.next().is_some() {
            return Err(ProbeError::Observation);
        }
        Ok(value)
    };
    let effective = |text: &str| -> Result<u32, ProbeError> {
        let mut fields = text.split_ascii_whitespace();
        let mut ids = [0_u32; 4];
        for id in &mut ids {
            *id = u32::try_from(number(fields.next().ok_or(ProbeError::Observation)?)?)
                .map_err(|_| ProbeError::Observation)?;
        }
        if fields.next().is_some() {
            return Err(ProbeError::Observation);
        }
        let [_, effective, _, _] = ids;
        Ok(effective)
    };
    let cap = field("CapEff")?;
    if cap.len() != 16 || !cap.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ProbeError::Observation);
    }
    let no_new_privileges = match field("NoNewPrivs")? {
        "0" => false,
        "1" => true,
        _ => return Err(ProbeError::Observation),
    };
    Ok(Status {
        uid: effective(field("Uid")?)?,
        gid: effective(field("Gid")?)?,
        capabilities: u64::from_str_radix(cap, 16).map_err(|_| ProbeError::Observation)?,
        no_new_privileges,
        seccomp: u8::try_from(number(field("Seccomp")?)?).map_err(|_| ProbeError::Observation)?,
    })
}
fn bounded(text: &str) -> Result<(), ProbeError> {
    if text.len() > 65536
        || text.lines().count() > 512
        || text.lines().any(|line| line.len() > 4096)
    {
        Err(ProbeError::Observation)
    } else {
        Ok(())
    }
}
pub(super) fn membership(text: &str, unified: bool) -> Result<bool, ProbeError> {
    bounded(text)?;
    let mut v2 = None;
    let mut required = [None; 3];
    for line in text.lines() {
        let mut fields = line.splitn(3, ':');
        let id = number(fields.next().ok_or(ProbeError::Observation)?)?;
        u32::try_from(id).map_err(|_| ProbeError::Observation)?;
        let controllers = fields.next().ok_or(ProbeError::Observation)?;
        let path = fields.next().ok_or(ProbeError::Observation)?;
        if controllers.len() > 256 || controllers.split(',').count() > 32 || path.is_empty() {
            return Err(ProbeError::Observation);
        }
        if controllers.is_empty() {
            if id != 0 {
                return Err(ProbeError::Observation);
            }
            if v2.replace(path == "/").is_some() {
                return Err(ProbeError::Observation);
            }
        } else if !unified {
            if id == 0 {
                return Err(ProbeError::Observation);
            }
            for controller in controllers.split(',') {
                let slot = match controller {
                    "memory" => required.get_mut(0),
                    "pids" => required.get_mut(1),
                    "cpu" => required.get_mut(2),
                    _ => None,
                };
                if let Some(slot) = slot {
                    if slot.replace(path == "/").is_some() {
                        return Err(ProbeError::Observation);
                    }
                }
            }
        }
    }
    if unified {
        v2.ok_or(ProbeError::Observation)
    } else {
        if required.iter().any(Option::is_none) {
            return Err(ProbeError::Observation);
        }
        Ok(required.iter().all(|value| *value == Some(true)))
    }
}
fn size(value: &str) -> Result<u64, ProbeError> {
    let (digits, scale) = match value.as_bytes().last() {
        Some(b'k') => (
            value.strip_suffix('k').ok_or(ProbeError::Observation)?,
            1024,
        ),
        Some(b'm') => (
            value.strip_suffix('m').ok_or(ProbeError::Observation)?,
            1024 * 1024,
        ),
        Some(b'g') => (
            value.strip_suffix('g').ok_or(ProbeError::Observation)?,
            1024 * 1024 * 1024,
        ),
        _ => (value, 1),
    };
    number(digits)?
        .checked_mul(scale)
        .ok_or(ProbeError::Observation)
}
pub(super) fn mounts(text: &str, mode: u32) -> Result<(bool, TemporaryReport), ProbeError> {
    bounded(text)?;
    let mut root = None;
    let mut temporary = None;
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        for _ in 0..4 {
            fields.next().ok_or(ProbeError::Observation)?;
        }
        let target = fields.next().ok_or(ProbeError::Observation)?;
        let options = fields.next().ok_or(ProbeError::Observation)?;
        if !matches!(target, "/" | "/tmp") {
            continue;
        }
        if !fields.by_ref().any(|field| field == "-") {
            return Err(ProbeError::Observation);
        }
        let filesystem = fields.next().ok_or(ProbeError::Observation)?;
        fields.next().ok_or(ProbeError::Observation)?;
        let super_options = fields.next().ok_or(ProbeError::Observation)?;
        if fields.next().is_some() {
            return Err(ProbeError::Observation);
        }
        if target == "/" {
            if root
                .replace(options.split(',').any(|v| v == "ro"))
                .is_some()
            {
                return Err(ProbeError::Observation);
            }
        } else {
            let mut sizes = super_options
                .split(',')
                .filter_map(|v| v.strip_prefix("size="));
            let bytes = size(sizes.next().ok_or(ProbeError::Observation)?)?;
            if sizes.next().is_some() {
                return Err(ProbeError::Observation);
            }
            let observed = TemporaryReport::builder()
                .bytes(bytes)
                .mode(mode)
                .tmpfs(filesystem == "tmpfs")
                .nosuid(options.split(',').any(|v| v == "nosuid"))
                .nodev(options.split(',').any(|v| v == "nodev"))
                .build();
            if temporary.replace(observed).is_some() {
                return Err(ProbeError::Observation);
            }
        }
    }
    Ok((
        root.ok_or(ProbeError::Observation)?,
        temporary.ok_or(ProbeError::Observation)?,
    ))
}

#[cfg(test)]
mod tests;
