//! # ducp-governance
//!
//! Reference-node binding governance is a **static parameter set** ([`Params`]) held as config
//! and set by the maintainer (spec/bindings/05 §4). At v1.0 these become
//! on-chain, role-chamber governance parameters; here they fix the economic
//! constants the ledger reads. They are **parameters, not invariants**.
//!
//! All rates are expressed in **parts-per-million** (ppm) so every computation is
//! exact integer arithmetic — no floats anywhere on a consensus path.
//!
//! Specification: <https://github.com/ducp-protocol/ducp-spec>
//! Status: Reference implementation for DUCP-SPEC v0.2.0.

use ducp_types::{Sp, Ucu, UCU_SCALE};
use serde::{Deserialize, Serialize};

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// One million — the ppm denominator.
pub const PPM: u128 = 1_000_000;

/// The devnet parameter set (spec/bindings/05 §4). Values are provisional and
/// tuned on devnet; they are config, not consensus invariants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Params {
    /// Work-issuance rate (ppm of metered 𝕌). Default 1% — below real resource cost
    /// (`I-SEC-WASHBOUND`).
    pub issuance_rate_ppm: u128,
    /// Per-task validator fee (ppm of `max_ucu`). Default 0.1%.
    pub fee_ppm: u128,
    /// Standing accrual rate (ppm of metered 𝕌). Default 1 SP per 𝕌 (1:1 at base scale).
    pub sp_rate_ppm: u128,
    /// Efficiency multiplier on Standing accrual (ppm). this binding sets 1.0 (no energy
    /// measured); ℚ is recorded but inert (`I-Q-REWARDNEUTRAL`).
    pub efficiency_mult_ppm: u128,
    /// Standing decay per epoch (ppm). Default 2%.
    pub decay_rate_ppm: u128,
    /// Claim-stake base (ppm of `max_ucu`). Default 50%.
    pub stake_base_ppm: u128,
    /// Maximum stake discount (ppm reduction). Default 80%.
    pub discount_max_ppm: u128,
    /// Standing at which the discount reaches its cap.
    pub sp_ref: Sp,
    /// Clawback window length in epochs. Default 32.
    pub clawback_epochs: u64,
    /// Audit sampling probability (ppm). Default 10%.
    pub audit_prob_ppm: u128,
    /// Fraud fine (ppm of payment `P`). Default 200% (2·P).
    pub fine_mult_ppm: u128,
    /// Challenger reward (ppm of fine `F`). Default 50% (F/2).
    pub challenger_reward_ppm: u128,
    /// Minimum challenge bond (ppm of payment `P`). Default 25%.
    pub bond_min_ppm: u128,
}

impl Default for Params {
    fn default() -> Self {
        Self::devnet()
    }
}

impl Params {
    /// The binding devnet defaults (spec/bindings/05 §4).
    pub const fn devnet() -> Self {
        Params {
            issuance_rate_ppm: 10_000,       // 1%
            fee_ppm: 1_000,                  // 0.1% of max_ucu
            sp_rate_ppm: 1_000_000,          // 1 SP per 𝕌
            efficiency_mult_ppm: 1_000_000,  // 1.0 (P0)
            decay_rate_ppm: 20_000,          // 2%
            stake_base_ppm: 500_000,         // 0.5 · max_ucu
            discount_max_ppm: 800_000,       // 0.8 cap
            sp_ref: 1_000 * UCU_SCALE as Sp, // provisional reference SP
            clawback_epochs: 32,
            audit_prob_ppm: 100_000,        // 0.10
            fine_mult_ppm: 2_000_000,       // 2·P
            challenger_reward_ppm: 500_000, // F/2
            bond_min_ppm: 250_000,          // 0.25·P
        }
    }

    /// Validator fee for a task with the given `max_ucu`.
    pub fn fee(&self, max_ucu: Ucu) -> Ucu {
        max_ucu * self.fee_ppm / PPM
    }

    /// Work-issuance minted for metered work `u`: `⌊issuance_rate · u⌋`.
    pub fn issuance(&self, u: Ucu) -> Ucu {
        u * self.issuance_rate_ppm / PPM
    }

    /// Standing accrual for metered work `u` under an efficiency multiplier (ppm):
    /// `⌊sp_rate · u · efficiency_mult⌋`. This binding passes `efficiency_mult = PPM`.
    pub fn standing_accrual(&self, u: Ucu, efficiency_mult_ppm: u128) -> Sp {
        (u * self.sp_rate_ppm / PPM * efficiency_mult_ppm / PPM) as Sp
    }

    /// Claim-stake base for a task with the given `max_ucu`.
    pub fn stake_base(&self, max_ucu: Ucu) -> Ucu {
        max_ucu * self.stake_base_ppm / PPM
    }

    /// The Standing-discounted claim stake a Provider must post.
    pub fn claim_stake(&self, max_ucu: Ucu, provider_sp: Sp) -> Ucu {
        let base = self.stake_base(max_ucu);
        let reduction = self.discount_reduction_ppm(provider_sp);
        base * (PPM - reduction) / PPM
    }

    /// The stake discount, as a ppm reduction, for a Provider's Standing:
    /// `min(discount_max, sp / sp_ref)`.
    pub fn discount_reduction_ppm(&self, provider_sp: Sp) -> u128 {
        if self.sp_ref <= 0 {
            return 0;
        }
        let sp_pos = provider_sp.max(0) as u128;
        (sp_pos * PPM / self.sp_ref as u128).min(self.discount_max_ppm)
    }

    /// Fraud fine for a payment `p`: `⌊fine_mult · p⌋` (default 2·p).
    pub fn fine(&self, p: Ucu) -> Ucu {
        p * self.fine_mult_ppm / PPM
    }

    /// Challenger reward out of a fine `f`: `⌊challenger_reward · f⌋` (default f/2).
    pub fn challenger_reward(&self, f: Ucu) -> Ucu {
        f * self.challenger_reward_ppm / PPM
    }

    /// Minimum acceptable challenge bond for a payment `p`: `⌊bond_min · p⌋`.
    pub fn bond_min(&self, p: Ucu) -> Ucu {
        p * self.bond_min_ppm / PPM
    }

    /// Decay Standing by one epoch: `⌊sp · (1 - decay_rate)⌋`.
    pub fn decay(&self, sp: Sp) -> Sp {
        let ppm_i = PPM as i128;
        sp * (ppm_i - self.decay_rate_ppm as i128) / ppm_i
    }
}

/// Source of protocol parameters. This binding uses [`StaticParams`] (a fixed set the
/// maintainer configures); at v1.0 an on-chain, role-chamber governance engine is a
/// later `impl ParamSource` with no change to the ledger (spec 07). This is the
/// governance seam.
pub trait ParamSource {
    /// The parameters in force for the current epoch.
    fn params(&self) -> Params;
}

/// The binding static parameter source.
#[derive(Debug, Clone, Copy, Default)]
pub struct StaticParams(pub Params);

impl ParamSource for StaticParams {
    fn params(&self) -> Params {
        self.0
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
    fn static_param_source_returns_devnet() {
        let src = StaticParams::default();
        assert_eq!(src.params(), Params::devnet());
    }

    #[test]
    fn devnet_defaults_match_spec() {
        let p = Params::devnet();
        // 1% issuance on 100 𝕌 → 1 𝕌.
        assert_eq!(p.issuance(100 * UCU_SCALE), UCU_SCALE);
        // fee 0.1% of 1000 𝕌 → 1 𝕌.
        assert_eq!(p.fee(1_000 * UCU_SCALE), UCU_SCALE);
        // 1 SP per 𝕌 at base scale, efficiency 1.0.
        assert_eq!(p.standing_accrual(5 * UCU_SCALE, PPM), 5 * UCU_SCALE as Sp);
        // stake base 0.5 · max_ucu.
        assert_eq!(p.stake_base(10 * UCU_SCALE), 5 * UCU_SCALE);
        // fine 2·P, reward F/2, bond 0.25·P.
        let f = p.fine(8 * UCU_SCALE);
        assert_eq!(f, 16 * UCU_SCALE);
        assert_eq!(p.challenger_reward(f), 8 * UCU_SCALE);
        assert_eq!(p.bond_min(8 * UCU_SCALE), 2 * UCU_SCALE);
    }

    #[test]
    fn efficiency_multiplier_is_one_in_p0() {
        let p = Params::devnet();
        assert_eq!(p.efficiency_mult_ppm, PPM);
        // ℚ-inertness at the accrual level: same 𝕌 → same accrual regardless of any
        // (hypothetical) efficiency value, because P0 always passes PPM.
        let u = 4 * UCU_SCALE;
        assert_eq!(p.standing_accrual(u, PPM), p.standing_accrual(u, PPM));
    }

    #[test]
    fn discount_is_capped() {
        let p = Params::devnet();
        // Very high Standing → capped at discount_max (0.8) → stake = 0.2 · base.
        let stake = p.claim_stake(10 * UCU_SCALE, 1_000_000 * UCU_SCALE as Sp);
        let base = p.stake_base(10 * UCU_SCALE);
        assert_eq!(stake, base * 200_000 / PPM);
    }

    #[test]
    fn decay_reduces_standing() {
        let p = Params::devnet();
        assert_eq!(p.decay(100 * UCU_SCALE as Sp), 98 * UCU_SCALE as Sp);
    }
}
