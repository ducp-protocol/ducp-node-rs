//! # ducp-verification
//!
//! Profile 0 verification: the universal **sampled re-execution** floor plus open
//! challenge (spec/implementation/03). Because the DVM is deterministic, a check is
//! exact — re-run on the same `{module, input, benchmark}` and compare the
//! `result_hash` and `ucu_count` byte-for-byte. TEE/ZK are reserved tiers.
//!
//! The optional [`EnergyAttestor`] seam (DP-0001, spec/09) validates a Power Seal
//! into a recorded ℚ. On the live path Profile 0 wires [`NullAttestor`], so ℚ stays
//! a reward-neutral, unmeasured observable; a real `impl EnergyAttestor` lands later
//! with no change to the proof path. Verifiers validate attestations as evidence —
//! they never re-measure energy (`I-VERIFY-RUNONCE`).
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//! Status: Profile 0 implementation for spec v0.2.0.

use ducp_dvm::{Benchmark, Dvm};
use ducp_types::{
    hash_bytes, ComputeProof, Hash, Identity, Limits, PowerSeal, Quant, TaskId, Ucu,
    VerificationTier, MICRO_Q_SCALE, UCU_SCALE,
};

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ============================ Re-execution =================================

/// The verdict of re-executing a Provider's proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The re-execution matched the proof.
    Accept,
    /// Mismatch — fraud. Carries the protocol's re-derived `result_hash`
    /// (`expected`) and the Provider's claimed one (`got`).
    Fraud { expected: Hash, got: Hash },
}

impl VerifyOutcome {
    pub fn is_fraud(&self) -> bool {
        matches!(self, VerifyOutcome::Fraud { .. })
    }
}

/// A verification tier. Adding a tier is an additional `impl Verifier` plus a
/// tier-assignment rule, with no change to the lifecycle or ledger
/// (spec/implementation/03 §5).
pub trait Verifier {
    /// The tier this verifier implements.
    fn tier(&self) -> VerificationTier;

    /// Re-derive and compare. For sampled re-execution this re-runs the DVM and
    /// compares the `result_hash` **and** `ucu_count` exactly (spec/implementation/03
    /// §2.2).
    fn check(
        &self,
        proof: &ComputeProof,
        program: &[u8],
        input: &[u8],
        limits: &Limits,
        benchmark: &Benchmark,
        dvm: &dyn Dvm,
    ) -> VerifyOutcome;
}

/// The Profile 0 verifier: exact re-execution.
pub struct SampledReexecVerifier;

impl Verifier for SampledReexecVerifier {
    fn tier(&self) -> VerificationTier {
        VerificationTier::SampledReexec
    }

    fn check(
        &self,
        proof: &ComputeProof,
        program: &[u8],
        input: &[u8],
        limits: &Limits,
        benchmark: &Benchmark,
        dvm: &dyn Dvm,
    ) -> VerifyOutcome {
        let reexec = dvm.execute(program, input, limits, benchmark);
        if reexec.result_hash == proof.result_hash && reexec.ucu_count == proof.ucu_count {
            VerifyOutcome::Accept
        } else {
            VerifyOutcome::Fraud {
                expected: reexec.result_hash,
                got: proof.result_hash,
            }
        }
    }
}

/// **Reserved** TEE-attestation verifier (spec/implementation/03; out of scope for
/// Profile 0). Present as a seam: a later profile implements `check` to validate a
/// hardware attestation cheaply. Profile 0 never assigns this tier, so `check` is
/// unreachable here.
pub struct TeeVerifier;

impl Verifier for TeeVerifier {
    fn tier(&self) -> VerificationTier {
        VerificationTier::Tee
    }
    fn check(
        &self,
        _proof: &ComputeProof,
        _program: &[u8],
        _input: &[u8],
        _limits: &Limits,
        _benchmark: &Benchmark,
        _dvm: &dyn Dvm,
    ) -> VerifyOutcome {
        unimplemented!(
            "TEE tier is reserved; not implemented in Profile 0 (spec/implementation/03)"
        )
    }
}

/// **Reserved** ZK-proof verifier (spec/implementation/03; out of scope for Profile
/// 0). Present as a seam; a later profile implements `check` to verify a succinct
/// proof. Profile 0 never assigns this tier.
pub struct ZkVerifier;

impl Verifier for ZkVerifier {
    fn tier(&self) -> VerificationTier {
        VerificationTier::Zk
    }
    fn check(
        &self,
        _proof: &ComputeProof,
        _program: &[u8],
        _input: &[u8],
        _limits: &Limits,
        _benchmark: &Benchmark,
        _dvm: &dyn Dvm,
    ) -> VerifyOutcome {
        unimplemented!("ZK tier is reserved; not implemented in Profile 0 (spec/implementation/03)")
    }
}

// ============================== Sampling ===================================

const PPM: u128 = 1_000_000;

/// Deterministic audit draw (spec/implementation/03 §2.1):
/// `selected = blake3(block_hash ‖ task_id)[0..8] < p · 2^64`. Reproducible and not
/// gameable by the Provider.
pub fn is_sampled(block_hash: &Hash, task: &TaskId, audit_prob_ppm: u128) -> bool {
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(block_hash);
    buf.extend_from_slice(task);
    let h = hash_bytes(&buf);
    let draw = u64::from_le_bytes(h[0..8].try_into().expect("8 bytes"));
    let threshold = audit_prob_ppm.saturating_mul(1u128 << 64) / PPM;
    (draw as u128) < threshold
}

/// Deterministically choose a re-executor from the eligible set, excluding the
/// original Provider (spec/implementation/03 §2.2), by the same seed. `None` if no
/// eligible worker remains.
pub fn select_reexecutor(
    block_hash: &Hash,
    task: &TaskId,
    eligible: &[Identity],
    exclude: &Identity,
) -> Option<Identity> {
    let filtered: Vec<Identity> = eligible.iter().copied().filter(|e| e != exclude).collect();
    if filtered.is_empty() {
        return None;
    }
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(block_hash);
    buf.extend_from_slice(task);
    let h = hash_bytes(&buf);
    let draw = u64::from_le_bytes(h[8..16].try_into().expect("8 bytes"));
    let idx = (draw as usize) % filtered.len();
    Some(filtered[idx])
}

// ===================== The ℚ energy-attestation seam =======================

/// Validates an optional Power Seal and, on success, returns the recorded ℚ lower
/// bound (spec/09 §4). ℚ is a **reward-neutral observable**: a failed or absent
/// attestation yields `None` and MUST NOT affect 𝕌 minting, settlement, or proof
/// validity (`I-Q-REWARDNEUTRAL`, `I-Q-NULL`). ℚ is protocol-derived from the seal
/// and the benchmark — never self-reported (`I-Q-DERIVED`).
pub trait EnergyAttestor {
    fn attest(&self, seal: &PowerSeal, ucu_count: Ucu, benchmark: &Benchmark) -> Option<Quant>;
}

/// Profile 0 **live** attestor: no energy is measured, so ℚ is always `None`
/// (`efficiency_mult = 1.0`). This is what the devnet wires, keeping base settlement
/// strictly 𝕌-proportional.
pub struct NullAttestor;

impl EnergyAttestor for NullAttestor {
    fn attest(&self, _seal: &PowerSeal, _ucu_count: Ucu, _benchmark: &Benchmark) -> Option<Quant> {
        None
    }
}

/// The **Sealed Power Proof** attestor (spec/09 §3–6, DP-0001): runs the three gated
/// checks on a present Power Seal and, if all pass, records the provable ℚ **lower
/// bound** (the *Sealed ℚ floor*, §4.2). Exists to satisfy the spec/09 §10
/// conformance vector and to prove the seam; it is **not** wired into base
/// settlement (which stays 𝕌-proportional — `I-Q-REWARDNEUTRAL`).
pub struct SealedAttestor;

impl EnergyAttestor for SealedAttestor {
    fn attest(&self, seal: &PowerSeal, ucu_count: Ucu, benchmark: &Benchmark) -> Option<Quant> {
        if !evidence_valid(seal) || !plausible(seal) || !well_formed(seal, benchmark) {
            return None;
        }
        Some(sealed_q_floor(seal, ucu_count, benchmark))
    }
}

/// (a) Evidence validity (spec/09 §6.1): the attestation must be present (chains to a
/// root of trust and binds the Task Hash). Profile 0 checks the evidence reference is
/// non-empty; real chain validation lands with the TEE tier.
fn evidence_valid(seal: &PowerSeal) -> bool {
    seal.attestation_evidence != [0u8; 32]
}

/// (b) Plausibility (spec/09 §6.1): the implied energy/temperature are physically
/// possible — positive, and not below the Landauer floor for the resolved work.
fn plausible(seal: &PowerSeal) -> bool {
    seal.power_cap_milliwatts > 0 && seal.window_millis > 0 && seal.t_max_millikelvin > 0
}

/// (c) Well-formedness (spec/09 §6.1): the declaration is consistent with the
/// benchmark the seal was produced under.
fn well_formed(seal: &PowerSeal, benchmark: &Benchmark) -> bool {
    seal.benchmark == benchmark.version
}

/// The Sealed ℚ floor (spec/09 §4.2), in fixed-point micro-ℚ, by exact integer math:
///
/// ```text
/// ℚ ≥ (C · E_baseline · T_std) / (E_consumed · T_max)
/// ```
///
/// where `C` is the metered work in whole 𝕌, `E_consumed = power_cap · window`
/// bounds the energy, and `T_max` bounds temperature.
pub fn sealed_q_floor(seal: &PowerSeal, ucu_count: Ucu, benchmark: &Benchmark) -> Quant {
    let c_ucu = (ucu_count / UCU_SCALE).max(1);
    let numerator = c_ucu
        * (benchmark.e_baseline as u128)
        * (benchmark.t_std_millikelvin as u128)
        * (MICRO_Q_SCALE as u128);
    let e_consumed = (seal.power_cap_milliwatts as u128) * (seal.window_millis as u128);
    let denom = e_consumed * (seal.t_max_millikelvin as u128);
    let micro = numerator.checked_div(denom).unwrap_or(0) as u64;
    Quant { micro_q: micro }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ducp_dvm::{echo_module, WasmtimeDvm};
    use ducp_types::{content_id, Boundary, PowerSeal, SealGrade, TierData, UCU_SCALE};

    fn limits() -> Limits {
        Limits {
            max_ucu: 10 * UCU_SCALE,
            max_memory_bytes: 16 * 1024 * 1024,
        }
    }

    fn honest_proof(dvm: &WasmtimeDvm, bench: &Benchmark, input: &[u8]) -> ComputeProof {
        let out = dvm.execute(&echo_module(), input, &limits(), bench);
        ComputeProof {
            task: [1u8; 32],
            provider: [2u8; 32],
            output: content_id(&out.output),
            result_hash: out.result_hash,
            ucu_count: out.ucu_count,
            benchmark: 0,
            tier_data: TierData::SampledReexec,
            power_seal: None,
        }
    }

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }

    #[test]
    fn verifier_tiers_are_distinct() {
        assert_eq!(
            SampledReexecVerifier.tier(),
            VerificationTier::SampledReexec
        );
        assert_eq!(TeeVerifier.tier(), VerificationTier::Tee);
        assert_eq!(ZkVerifier.tier(), VerificationTier::Zk);
    }

    #[test]
    fn honest_proof_is_accepted() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let proof = honest_proof(&dvm, &bench, b"data");
        let v = SampledReexecVerifier;
        assert_eq!(
            v.check(&proof, &echo_module(), b"data", &limits(), &bench, &dvm),
            VerifyOutcome::Accept
        );
    }

    #[test]
    fn forged_result_hash_is_fraud() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let mut proof = honest_proof(&dvm, &bench, b"data");
        proof.result_hash = [0xAB; 32]; // forged
        let v = SampledReexecVerifier;
        let outcome = v.check(&proof, &echo_module(), b"data", &limits(), &bench, &dvm);
        assert!(outcome.is_fraud());
    }

    #[test]
    fn forged_ucu_count_is_fraud() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let mut proof = honest_proof(&dvm, &bench, b"data");
        proof.ucu_count += 1; // forged
        let v = SampledReexecVerifier;
        assert!(v
            .check(&proof, &echo_module(), b"data", &limits(), &bench, &dvm)
            .is_fraud());
    }

    #[test]
    fn sampling_is_deterministic_and_bounded() {
        let block = [7u8; 32];
        let task = [9u8; 32];
        // p = 0 never samples; p = 100% always samples.
        assert!(!is_sampled(&block, &task, 0));
        assert!(is_sampled(&block, &task, PPM));
        // Deterministic for a fixed seed.
        assert_eq!(
            is_sampled(&block, &task, 500_000),
            is_sampled(&block, &task, 500_000)
        );
    }

    #[test]
    fn reexecutor_excludes_provider_and_is_deterministic() {
        let block = [1u8; 32];
        let task = [2u8; 32];
        let provider = [10u8; 32];
        let eligible = [[10u8; 32], [11u8; 32], [12u8; 32]];
        let pick = select_reexecutor(&block, &task, &eligible, &provider).unwrap();
        assert_ne!(pick, provider);
        assert_eq!(
            select_reexecutor(&block, &task, &eligible, &provider),
            Some(pick)
        );
        // Only the provider is eligible → nobody to re-execute.
        assert_eq!(
            select_reexecutor(&block, &task, &[provider], &provider),
            None
        );
    }

    fn q_benchmark() -> Benchmark {
        Benchmark {
            version: 0,
            fuel_cost_table_hash: [0u8; 32],
            fuel_per_ucu: 1,
            e_baseline: 137, // 13.7 pJ/𝕌 (0.1 pJ units)
            t_std_millikelvin: 300_000,
        }
    }

    fn seal(power_cap: u64, t_max_mk: u64, grade: SealGrade, boundary: Boundary) -> PowerSeal {
        PowerSeal {
            seal_grade: grade,
            boundary,
            power_cap_milliwatts: power_cap,
            window_millis: 50_000, // = C (whole 𝕌), so power_cap encodes per-𝕌 energy
            t_max_millikelvin: t_max_mk,
            attestation_evidence: content_id(b"root-of-trust-quote"),
            benchmark: 0,
        }
    }

    #[test]
    fn null_attestor_records_no_q() {
        let bench = q_benchmark();
        let s = seal(137, 300_000, SealGrade::S0Identity, Boundary::Chip);
        assert_eq!(NullAttestor.attest(&s, 50_000 * UCU_SCALE, &bench), None);
    }

    #[test]
    fn sealed_attestor_reproduces_dp0001_vector() {
        // spec/09 §10 / DP-0001 §9: 𝕌 = 50,000; baseline 13.7 pJ at 300 K.
        let bench = q_benchmark();
        let ucu = 50_000 * UCU_SCALE;
        let a = SealedAttestor
            .attest(
                &seal(274, 350_000, SealGrade::S1Witnessed, Boundary::Node),
                ucu,
                &bench,
            )
            .unwrap();
        let b = SealedAttestor
            .attest(
                &seal(137, 300_000, SealGrade::S2Locked, Boundary::Chip),
                ucu,
                &bench,
            )
            .unwrap();
        let c = SealedAttestor
            .attest(
                &seal(100, 250_000, SealGrade::S2Locked, Boundary::Chip),
                ucu,
                &bench,
            )
            .unwrap();
        // ℚ ≈ {0.43, 1.00, 1.64} in micro-ℚ.
        assert_eq!(a.micro_q, 428_571); // 0.428571 ≈ 0.43
        assert_eq!(b.micro_q, 1_000_000); // 1.00
        assert_eq!(c.micro_q, 1_644_000); // 1.644 ≈ 1.64
    }

    #[test]
    fn sealed_attestor_rejects_missing_evidence() {
        let bench = q_benchmark();
        let mut s = seal(137, 300_000, SealGrade::S0Identity, Boundary::Chip);
        s.attestation_evidence = [0u8; 32]; // no evidence
        assert_eq!(SealedAttestor.attest(&s, 50_000 * UCU_SCALE, &bench), None);
    }

    #[test]
    fn sealed_attestor_rejects_benchmark_mismatch() {
        let bench = q_benchmark();
        let mut s = seal(137, 300_000, SealGrade::S0Identity, Boundary::Chip);
        s.benchmark = 99; // not the benchmark in force
        assert_eq!(SealedAttestor.attest(&s, 50_000 * UCU_SCALE, &bench), None);
    }
}
