//! # ducp-types
//!
//! Canonical DUCP data model shared by every conforming node — identifiers, the
//! Compute Proof, and the **Quant (ℚ)** efficiency observable. Field shapes follow
//! the Profile 0 specification (`spec/implementation/01-data-model.md`); canonical
//! byte encoding (`borsh`) and hashing (BLAKE3) are added later and are not yet
//! derived here.
//!
//! This scaffold lands the identifiers, the Compute Proof (with its optional
//! [`PowerSeal`]), and the ℚ types introduced by **DP-0001** and **spec/09** as a
//! reward-neutral observable. The remaining records (`Submission`, `Receipt`,
//! `Account`, `State`) are added as the node grows.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//!
//! Status: scaffold for spec v0.2.0 — data shapes only, not yet operational.

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ----- Identifiers (spec/implementation/01 §2) -----

pub type Hash = [u8; 32];
pub type Identity = [u8; 32]; // Ed25519 public key (Profile 0)
pub type ContentId = Hash; // hash of an off-ledger payload
pub type TaskId = Hash;
pub type Ucu = u128; // base units; 1 𝕌 = 1_000_000_000
pub type Sp = i128; // Standing points, base scale
pub type BenchmarkVersion = u32;

// ----- Verification evidence (spec/implementation/01 §4) -----

/// Verification tier, assigned by the DVM at submit — never chosen (`I-VERIFY-NOCHOICE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationTier {
    SampledReexec, // Profile 0
    Tee,
    Zk,
}

/// Tier-specific evidence carried by a [`ComputeProof`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierData {
    SampledReexec,                // Profile 0: determinism enables exact re-check
    Tee { attestation: Vec<u8> }, // reserved
    Zk { proof: Vec<u8> },        // reserved
}

/// The Provider's evidence for a settled task (spec/implementation/01 §4).
///
/// The optional [`power_seal`](ComputeProof::power_seal) is the energy attestation
/// introduced by **DP-0001**: absent by default, it never affects `ucu_count`,
/// minting, or proof validity (`I-Q-REWARDNEUTRAL`). When absent, the task's ℚ is
/// `None` (`I-Q-NULL`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputeProof {
    pub task: TaskId,
    pub provider: Identity,
    pub output: ContentId,
    pub result_hash: Hash,
    pub ucu_count: Ucu,
    pub benchmark: BenchmarkVersion,
    pub tier_data: TierData,
    /// Optional, reward-neutral energy attestation (DP-0001, spec/09). `None` in Profile 0.
    pub power_seal: Option<PowerSeal>,
}

// ----- The efficiency observable ℚ (DP-0001, spec/09) -----

/// Strength of the power-cap attestation behind a [`PowerSeal`] (spec/09 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealGrade {
    /// Self-attested static power cap. Available today; weakest evidence.
    S0Identity,
    /// Out-of-band, root-of-trust–signed cap or meter (e.g. BMC / smart PDU).
    S1Witnessed,
    /// Vendor-locked, signed on-die power register. Strongest; not yet available.
    S2Locked,
}

/// Where energy was bounded/measured (spec/09 §5.2). The protocol never fixes a
/// single boundary; it records the declared one so ℚ is compared only within an
/// identical `(grade, boundary)` (`I-Q-COMPARE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    Chip,
    Node,
    Facility,
}

/// A recorded ℚ value as fixed-point **micro-ℚ** (ℚ × 1_000_000).
///
/// Integer by construction: no floats appear in any hashed structure
/// (spec/implementation/01 §1). ℚ = 1.0 (`micro_q == 1_000_000`) is frontier-grade;
/// below 1.0 is behind the frontier, above 1.0 is ahead of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quant {
    pub micro_q: u64,
}

impl Quant {
    /// Frontier-grade efficiency, ℚ = 1.0.
    pub const ONE: Quant = Quant { micro_q: 1_000_000 };
}

/// Optional energy attestation on a [`ComputeProof`] (spec/09 §3).
///
/// It attests *configuration* (a static power cap), not data-dependent telemetry,
/// so it is side-channel-safe and signable by existing roots of trust. All fields
/// are integers — no floats in hashed data. The recorded ℚ is the *Sealed* lower
/// bound `≥ (C · E_baseline · T_std) / (power_cap · window · T_max)` (spec/09 §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerSeal {
    pub seal_grade: SealGrade,
    pub boundary: Boundary,
    pub power_cap_milliwatts: u64,
    pub window_millis: u64,
    pub t_max_millikelvin: u64,
    /// Evidence chaining the seal to a hardware root of trust and to the Task Hash;
    /// bulky evidence lives off-ledger, referenced by content id.
    pub attestation_evidence: ContentId,
    /// Benchmark epoch supplying `E_baseline × T_std` used to compute ℚ.
    pub benchmark: BenchmarkVersion,
}

/// One entry of the on-chain **ℚ-ledger** (spec/09 §7): the `(𝕌, ℚ)` pair recorded
/// for every settled task. `q` is `None` wherever energy was not validly attested
/// (`I-Q-NULL`); recording it never affects settlement (`I-Q-REWARDNEUTRAL`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QLedgerEntry {
    pub task: TaskId,
    pub ucu: Ucu,
    pub q: Option<Quant>,
    pub seal_grade: Option<SealGrade>,
    pub boundary: Option<Boundary>,
    pub benchmark: BenchmarkVersion,
}

impl QLedgerEntry {
    /// A reward-neutral entry with no energy attestation — the Profile 0 default for
    /// every task: 𝕌 recorded, ℚ unmeasured (`I-Q-NULL`).
    pub fn unmeasured(task: TaskId, ucu: Ucu, benchmark: BenchmarkVersion) -> Self {
        QLedgerEntry {
            task,
            ucu,
            q: None,
            seal_grade: None,
            boundary: None,
            benchmark,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }

    #[test]
    fn profile0_proof_carries_no_power_seal() {
        let proof = ComputeProof {
            task: [0u8; 32],
            provider: [0u8; 32],
            output: [0u8; 32],
            result_hash: [0u8; 32],
            ucu_count: 4_000_000_000, // 4 𝕌
            benchmark: 0,
            tier_data: TierData::SampledReexec,
            power_seal: None,
        };
        assert!(proof.power_seal.is_none());
    }

    #[test]
    fn unmeasured_entry_is_reward_neutral_null() {
        let e = QLedgerEntry::unmeasured([1u8; 32], 4_000_000_000, 0);
        assert_eq!(e.q, None);
        assert_eq!(e.seal_grade, None);
        assert_eq!(e.ucu, 4_000_000_000); // 𝕌 present regardless of ℚ
    }

    #[test]
    fn quant_one_is_frontier_grade() {
        assert_eq!(Quant::ONE.micro_q, 1_000_000);
    }
}
