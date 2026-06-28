//! # ducp-verification
//!
//! Layered verification: TEE attestation, ZK proofs, and sampled re-execution. Part of the DUCP reference node.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//!
//! Status: scaffold for spec v0.1.0 — not yet implemented.
//!
//! The [`EnergyAttestor`] seam (DP-0001, spec/09) validates the optional Power Seal
//! into a recorded ℚ. Profile 0 measures no energy, so [`NullAttestor`] always
//! returns `None` — ℚ stays a reward-neutral, unmeasured observable.

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Validates an optional Power Seal and, on success, returns the recorded ℚ lower
/// bound (spec/09 §4). ℚ is a **reward-neutral observable**: a failed or absent
/// attestation yields `None` and MUST NOT affect 𝕌 minting, settlement, or proof
/// validity (`I-Q-REWARDNEUTRAL`, `I-Q-NULL`). Verifiers validate the attestation;
/// they never re-measure energy (`I-VERIFY-RUNONCE`).
pub trait EnergyAttestor {
    fn attest(&self, seal: &ducp_types::PowerSeal) -> Option<ducp_types::Quant>;
}

/// Profile 0 attestor: no energy is measured, so ℚ is always `None`
/// (`efficiency_mult = 1.0`; see `ducp-ledger`). Real attestation — a TEE-carried
/// reading, a signed meter, or a locked register — lands later as additional
/// `impl EnergyAttestor`s, with no change to the proof path.
pub struct NullAttestor;

impl EnergyAttestor for NullAttestor {
    fn attest(&self, _seal: &ducp_types::PowerSeal) -> Option<ducp_types::Quant> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{EnergyAttestor, NullAttestor};
    use ducp_types::{Boundary, PowerSeal, SealGrade};

    #[test]
    fn version_is_set() {
        assert!(!super::version().is_empty());
    }

    #[test]
    fn null_attestor_records_no_q() {
        let seal = PowerSeal {
            seal_grade: SealGrade::S0Identity,
            boundary: Boundary::Chip,
            power_cap_milliwatts: 300_000,
            window_millis: 1_000,
            t_max_millikelvin: 350_000,
            attestation_evidence: [0u8; 32],
            benchmark: 0,
        };
        // Even a well-formed seal yields no ℚ in Profile 0 — reward-neutral and inert.
        assert_eq!(NullAttestor.attest(&seal), None);
    }
}
