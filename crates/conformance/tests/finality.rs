//! M5 conformance: clawback-window finality (spec/bindings/04 §3).
//!
//! After the clawback window closes with no successful challenge, the claim stake is
//! released while the settled Receipt stays immutable (`I-ECON-FINAL`).

use ducp_conformance::{finality_record, load_json, FinalityRecord};

fn committed() -> FinalityRecord {
    load_json("settlement", "finality.json")
}

#[test]
fn committed_matches_reference() {
    assert_eq!(committed(), finality_record());
}

#[test]
fn stake_released_and_receipt_immutable() {
    let r = committed();
    assert!(r.released, "claim stake released after the window");
    assert_eq!(r.provider.bonded, 0, "bond returned to spendable balance");
    assert!(r.receipt_unchanged, "settled Receipt is never rewritten");
    assert!(r.conserved);
}
