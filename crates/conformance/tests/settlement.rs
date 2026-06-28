//! M2/M3 conformance: settlement golden vector (spec/implementation/04 §3).
//!
//! Pins the post-state of a happy-path settlement and checks the cross-cutting
//! invariants on the published vector: conservation (`I-LEDGER-CONSERVE`) and the
//! reward-neutral (𝕌, ℚ) entry with ℚ null in Profile 0 (`I-Q-NULL`).

use ducp_conformance::{load_json, settlement_record, SettlementRecord};

fn committed() -> SettlementRecord {
    load_json("settlement", "happy_path.json")
}

#[test]
fn committed_matches_reference_ledger() {
    assert_eq!(committed(), settlement_record());
}

#[test]
fn q_entry_is_reward_neutral_null() {
    let r = committed();
    assert!(
        r.q_entry.q.is_none(),
        "ℚ MUST be null with no Power Seal (I-Q-NULL)"
    );
    assert!(r.q_entry.seal_grade.is_none());
    assert!(r.q_entry.boundary.is_none());
    // The 𝕌 of the pair equals the paid amount — the (𝕌, ℚ) record.
    assert_eq!(r.q_entry.ucu, r.receipt.paid_to_provider);
}

#[test]
fn escrow_drained_and_stake_bonded_for_clawback() {
    let r = committed();
    assert_eq!(
        r.requester.escrowed, 0,
        "escrow fully released at settlement"
    );
    assert!(r.provider.bonded > 0, "claim stake remains bonded");
    assert_eq!(
        r.receipt.clawback_until, 32,
        "bond locked for the clawback window"
    );
}

#[test]
fn conservation_holds_on_published_vector() {
    // I-LEDGER-CONSERVE: Σ(balance + escrowed + bonded) + fee_pool == minted − burned.
    let r = committed();
    let held = r.requester.balance
        + r.requester.escrowed
        + r.requester.bonded
        + r.provider.balance
        + r.provider.escrowed
        + r.provider.bonded
        + r.fee_pool.parse::<u128>().unwrap();
    let minted = r.minted.parse::<u128>().unwrap();
    assert_eq!(held, minted);
}
