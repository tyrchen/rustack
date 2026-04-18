//! Cache-behavior path-pattern matching.

use rustack_cloudfront_model::{CacheBehavior, DistributionConfig};

/// Select the cache behavior for a given request path.
///
/// Iterates `CacheBehaviors` in order; first match wins. Falls back to
/// `DefaultCacheBehavior`.
#[must_use]
pub fn select_behavior<'a>(config: &'a DistributionConfig, path: &str) -> &'a CacheBehavior {
    for cb in &config.cache_behaviors {
        if !cb.path_pattern.is_empty() && matches_pattern(&cb.path_pattern, path) {
            return cb;
        }
    }
    &config.default_cache_behavior
}

/// Glob match: `*` matches any sequence, `?` matches one character.
/// All other bytes are literal. Case-sensitive.
#[must_use]
pub fn matches_pattern(pattern: &str, path: &str) -> bool {
    glob_match(pattern.as_bytes(), path.as_bytes())
}

fn glob_match(pattern: &[u8], s: &[u8]) -> bool {
    let (mut p, mut i) = (0usize, 0usize);
    let mut last_star: Option<usize> = None;
    let mut last_match = 0usize;
    while i < s.len() {
        match pattern.get(p) {
            Some(&b'*') => {
                last_star = Some(p);
                last_match = i;
                p += 1;
            }
            Some(&b'?') => {
                p += 1;
                i += 1;
            }
            Some(&c) if c == s[i] => {
                p += 1;
                i += 1;
            }
            _ => match last_star {
                Some(sp) => {
                    p = sp + 1;
                    last_match += 1;
                    i = last_match;
                }
                None => return false,
            },
        }
    }
    while pattern.get(p) == Some(&b'*') {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert!(matches_pattern("/robots.txt", "/robots.txt"));
        assert!(!matches_pattern("/robots.txt", "/Robots.txt"));
    }

    #[test]
    fn test_suffix_star() {
        assert!(matches_pattern("/images/*", "/images/foo.png"));
        assert!(matches_pattern("/images/*", "/images/"));
        assert!(matches_pattern("/images/*", "/images/a/b/c"));
        assert!(!matches_pattern("/images/*", "/assets/foo.png"));
    }

    #[test]
    fn test_mid_star() {
        assert!(matches_pattern("*.jpg", "/images/foo.jpg"));
        assert!(!matches_pattern("*.jpg", "/images/foo.png"));
    }

    #[test]
    fn test_question() {
        assert!(matches_pattern("/?/foo", "/a/foo"));
        assert!(!matches_pattern("/?/foo", "//foo"));
    }
}
