//! # ducp-ledger
//!
//! Settlement of UCU and the Standing reputation ledger. Part of the DUCP reference node.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//!
//! Status: scaffold for spec v0.1.0 — not yet implemented.
//!
//! Settlement keeps base reward strictly 𝕌-proportional. The efficiency multiplier
//! (DP-0001, spec/09) is the only place ℚ could touch accrual — and in Profile 0 it
//! is fixed at 1.0, so ℚ is recorded but inert (`I-Q-REWARDNEUTRAL`).

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Profile 0 efficiency multiplier on Standing accrual, as an exact integer ratio
/// `num/den` (no floats in ledger math). P0 measures no energy, so it is
/// **1/1 = 1.0** (spec/implementation/04 §3.5). ℚ never scales 𝕌 minting or
/// settlement — only, once measurement exists, Standing accrual and routing.
pub const EFFICIENCY_MULT_NUM: u64 = 1;
pub const EFFICIENCY_MULT_DEN: u64 = 1;

/// Standing accrual from `u` 𝕌 base units: `⌊sp_rate · u · efficiency_mult⌋`
/// (spec/implementation/04 §3.5). In Profile 0 `efficiency_mult == 1`.
pub fn standing_accrual(sp_rate: u64, u: ducp_types::Ucu) -> ducp_types::Sp {
    let scaled =
        u * sp_rate as u128 * EFFICIENCY_MULT_NUM as u128 / EFFICIENCY_MULT_DEN as u128;
    scaled as ducp_types::Sp
}

/// Build the reward-neutral ℚ-ledger entry for a settled task in Profile 0:
/// 𝕌 is recorded; ℚ is null (no energy measured) — `I-Q-REWARDNEUTRAL`, `I-Q-NULL`.
pub fn q_ledger_entry_p0(
    task: ducp_types::TaskId,
    ucu: ducp_types::Ucu,
    benchmark: ducp_types::BenchmarkVersion,
) -> ducp_types::QLedgerEntry {
    ducp_types::QLedgerEntry::unmeasured(task, ucu, benchmark)
}

#[cfg(test)]
mod tests {
    use ducp_types::Quant;

    #[test]
    fn version_is_set() {
        assert!(!super::version().is_empty());
    }

    #[test]
    fn p0_efficiency_mult_is_one() {
        assert_eq!(super::EFFICIENCY_MULT_NUM, super::EFFICIENCY_MULT_DEN); // 1.0
    }

    #[test]
    fn q_does_not_enter_accrual_in_p0() {
        // Same 𝕌 → same accrual whether ℚ is null or a value (reward-neutral).
        let u: ducp_types::Ucu = 4_000_000_000; // 4 𝕌
        let null_entry = super::q_ledger_entry_p0([0u8; 32], u, 0);
        let mut valued = super::q_ledger_entry_p0([0u8; 32], u, 0);
        valued.q = Some(Quant::ONE);
        assert_eq!(null_entry.q, None);
        assert_eq!(valued.q, Some(Quant::ONE));
        // Accrual depends on 𝕌 only:
        assert_eq!(
            super::standing_accrual(1, null_entry.ucu),
            super::standing_accrual(1, valued.ucu)
        );
        assert_eq!(super::standing_accrual(1, u), 4_000_000_000);
    }
}
