//! Version staging label constants and utilities.

/// The staging label for the current active version.
pub const AWSCURRENT: &str = "AWSCURRENT";

/// The staging label for the previous version (automatically managed).
pub const AWSPREVIOUS: &str = "AWSPREVIOUS";

/// The staging label for a version pending rotation.
pub const AWSPENDING: &str = "AWSPENDING";

/// Maximum number of versions to keep per secret (including deprecated).
pub const MAX_VERSIONS: usize = 100;
