//! # ducp-governance
//!
//! Reputation-weighted, role-chamber governance. Part of the DUCP reference node.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//!
//! Status: scaffold for spec v0.1.0 — not yet implemented.

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_set() {
        assert!(!super::version().is_empty());
    }
}
