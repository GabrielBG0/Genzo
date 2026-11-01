//! Contract versioning.
//!
//! Every change to a type exported to `contract/` bumps [`CONTRACT_VERSION`].
//! The snapshot test in `tests/contract_snapshot.rs` fails when the generated
//! TypeScript changes and the version does not.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Version of the engine contract, as `major.minor.patch`.
///
/// While the major version is 0, a minor bump is a breaking change.
/// From 1.0 on, a major bump is.
pub const CONTRACT_VERSION: &str = "0.5.1";

/// A parsed contract version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct ContractVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl ContractVersion {
    /// The version this build of the engine speaks.
    pub fn current() -> Self {
        Self::parse(CONTRACT_VERSION).expect("CONTRACT_VERSION is a valid version")
    }

    /// Parses `major.minor.patch`. Returns `None` on anything else.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    /// Whether a frontend built against `self` can talk to an engine at `other`.
    ///
    /// Same major version, and while major is 0, the same minor version too.
    /// Patch versions never break compatibility.
    pub fn is_compatible_with(self, other: Self) -> bool {
        if self.major != other.major {
            return false;
        }
        self.major != 0 || self.minor == other.minor
    }
}

impl std::fmt::Display for ContractVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> ContractVersion {
        ContractVersion::parse(s).unwrap()
    }

    #[test]
    fn parses_current() {
        assert_eq!(ContractVersion::current().to_string(), CONTRACT_VERSION);
    }

    #[test]
    fn rejects_malformed() {
        for s in ["", "1", "1.2", "1.2.3.4", "a.b.c", "1.-2.3"] {
            assert!(ContractVersion::parse(s).is_none(), "{s}");
        }
    }

    #[test]
    fn compatibility_rules() {
        assert!(v("0.1.0").is_compatible_with(v("0.1.7")));
        assert!(!v("0.1.0").is_compatible_with(v("0.2.0")));
        assert!(v("1.0.0").is_compatible_with(v("1.4.2")));
        assert!(!v("1.0.0").is_compatible_with(v("2.0.0")));
    }
}
