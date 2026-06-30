//! M6 conformance: state-machine replication (spec/bindings/04 §6).
//!
//! Producing then replaying a block sequence reaches the identical `state_root`, so
//! every node converges on the same ledger state even with a single proposer.

use ducp_conformance::{load_json, replication_record, ReplicationRecord};

fn committed() -> ReplicationRecord {
    load_json("replication", "blocks.json")
}

#[test]
fn committed_matches_reference() {
    assert_eq!(committed(), replication_record());
}

#[test]
fn replica_reaches_identical_state_root() {
    let r = committed();
    assert!(r.replica_matches, "replica diverged from the proposer");
    assert!(r.blocks >= 3);
    assert_eq!(
        r.block_state_roots.last().map(String::as_str),
        Some(r.final_state_root.as_str())
    );
}
