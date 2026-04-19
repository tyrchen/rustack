//! Host-header routing.

use std::sync::Arc;

use rustack_cloudfront_core::RustackCloudFront;

/// Return the matched distribution ID if `host` identifies a known distribution.
///
/// Matching rules:
/// 1. `{lowercased-id}.{suffix}` where `{id}` is a known distribution.
/// 2. Exact match against any distribution's `Aliases.Items`.
#[must_use]
pub fn match_host(
    provider: &Arc<RustackCloudFront>,
    domain_suffix: &str,
    host: &str,
) -> Option<String> {
    // Strip port if present.
    let host_no_port = host.split_once(':').map_or(host, |(h, _)| h);

    // 1. {id}.{suffix}
    if let Some(sub) = host_no_port.strip_suffix(domain_suffix) {
        let sub = sub.trim_end_matches('.');
        if !sub.is_empty() && !sub.contains('.') {
            let uppercase = sub.to_ascii_uppercase();
            if provider.store().distributions.contains_key(&uppercase) {
                return Some(uppercase);
            }
        }
    }

    // 2. Aliases.
    for e in provider.store().distributions.iter() {
        if e.value()
            .config
            .aliases
            .iter()
            .any(|a| a.eq_ignore_ascii_case(host_no_port))
        {
            return Some(e.key().clone());
        }
    }
    None
}
