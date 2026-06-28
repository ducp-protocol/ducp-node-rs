//! ℚ-observable conformance: the DP-0001 §9 / spec/09 §10 test vector.
//!
//! Four Providers run one task (𝕌 = 50,000) against a 13.7 pJ baseline at 300 K.
//! Each MUST receive identical 𝕌 and identical payment, recording ℚ ≈
//! {0.43, 1.00, 1.64, null} — reward-neutral by construction.

use ducp_conformance::{load_json, q_observable_record, QObservableRecord};

fn committed() -> QObservableRecord {
    load_json("q-observable", "dp0001.json")
}

#[test]
fn committed_matches_reference() {
    assert_eq!(committed(), q_observable_record());
}

#[test]
fn q_values_match_the_vector() {
    let r = committed();
    let q = |label: &str| {
        r.providers
            .iter()
            .find(|p| p.label == label)
            .unwrap()
            .micro_q
    };
    assert_eq!(q("A"), Some(428_571)); // 0.43
    assert_eq!(q("B"), Some(1_000_000)); // 1.00
    assert_eq!(q("C"), Some(1_644_000)); // 1.64
    assert_eq!(q("D"), None); // no Power Seal → ℚ null (I-Q-NULL)
}

#[test]
fn all_payments_are_identical_reward_neutral() {
    let r = committed();
    assert!(
        r.all_payments_equal,
        "ℚ MUST NOT change payment (I-Q-REWARDNEUTRAL)"
    );
    // Every Provider paid the same 𝕌 regardless of ℚ (including the null one).
    for p in &r.providers {
        assert_eq!(p.paid_ucu, r.ucu);
    }
}

#[test]
fn q_compared_only_within_grade_and_boundary() {
    // I-Q-COMPARE: B and C share (S2, chip) and are comparable; A is (S1, node).
    let r = committed();
    let row = |label: &str| {
        r.providers
            .iter()
            .find(|p| p.label == label)
            .unwrap()
            .clone()
    };
    let b = row("B");
    let c = row("C");
    assert_eq!(b.seal_grade, c.seal_grade);
    assert_eq!(b.boundary, c.boundary);
    assert!(c.micro_q > b.micro_q); // C is more efficient within the same class
}
