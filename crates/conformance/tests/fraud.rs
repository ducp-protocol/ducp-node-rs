//! M4/M5 conformance: fraud golden vector (spec/implementation/03 §4, 04 §4).
//!
//! A forged proof is settled optimistically, challenged, re-executed, and slashed.
//! Pins the post-state and verifies conservation across the fraud path.

use ducp_conformance::{fraud_record, load_json, FraudRecord};

fn committed() -> FraudRecord {
    load_json("fraud", "challenge.json")
}

#[test]
fn committed_matches_reference() {
    assert_eq!(committed(), fraud_record());
}

#[test]
fn re_execution_detects_the_forgery() {
    let r = committed();
    assert!(r.verdict_is_fraud);
    assert_ne!(r.forged_result_hash, r.true_result_hash);
}

#[test]
fn penalties_applied_and_standing_floored() {
    let r = committed();
    assert_eq!(r.provider_standing.sp, 0, "Standing floored on fraud");
    assert_eq!(r.provider_standing.strikes, 1);
    // Offsetting burn (W) + fine remainder both leave supply.
    assert_ne!(r.burned, "0", "work-issuance + fine remainder burned");
}

#[test]
fn conservation_holds_across_fraud_path() {
    let r = committed();
    assert!(r.conserved);
    let held = r.requester.balance
        + r.requester.escrowed
        + r.requester.bonded
        + r.provider.balance
        + r.provider.escrowed
        + r.provider.bonded
        + r.challenger.balance
        + r.challenger.escrowed
        + r.challenger.bonded
        + r.fee_pool.parse::<u128>().unwrap();
    let circulating = r.minted.parse::<u128>().unwrap() - r.burned.parse::<u128>().unwrap();
    assert_eq!(held, circulating, "I-LEDGER-CONSERVE across fraud");
}
