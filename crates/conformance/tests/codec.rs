//! M0 conformance: codec / hash golden vectors (spec/bindings/01 §7).
//!
//! Pins the canonical (borsh) bytes and BLAKE3-256 hashes of the core data model,
//! including the ℚ types. Regenerate with `cargo run -p ducp-conformance --bin
//! gen-vectors` after an intentional schema change.

use ducp_conformance::{codec_records, load_json, unhex, CodecRecord};
use ducp_types::{Block, ComputeProof, QLedgerEntry, SignedTx, TaskBody};

fn committed() -> Vec<CodecRecord> {
    load_json("codec", "types.json")
}

#[test]
fn committed_vectors_match_reference_crate() {
    // Golden regression: the published file MUST equal what the crates produce now.
    assert_eq!(
        committed(),
        codec_records(),
        "codec vectors drifted from the reference crate; regenerate with gen-vectors if intended"
    );
}

#[test]
fn hash_is_blake3_of_canonical_bytes() {
    for r in committed() {
        let canonical = unhex(&r.canonical_hex);
        let expected = hex::encode(ducp_types::hash_bytes(&canonical));
        assert_eq!(r.hash, expected, "hash mismatch for {}", r.name);
    }
}

#[test]
fn published_value_decodes_and_reencodes_to_canonical() {
    // The serde wire form (hex/decimal strings) MUST deserialize and re-encode to
    // the exact canonical bytes — i.e. the JSON-RPC form is faithful to the codec.
    for r in committed() {
        let canonical = match r.name.as_str() {
            "task_body" => roundtrip::<TaskBody>(&r),
            "compute_proof_no_seal" | "compute_proof_with_seal" => roundtrip::<ComputeProof>(&r),
            "signed_transfer" => roundtrip::<SignedTx>(&r),
            "q_entry_null" | "q_entry_valued" => roundtrip::<QLedgerEntry>(&r),
            "block" => roundtrip::<Block>(&r),
            _ => continue, // covered by the golden-equality test above
        };
        assert_eq!(
            canonical, r.canonical_hex,
            "re-encode mismatch for {}",
            r.name
        );
    }
}

fn roundtrip<T>(r: &CodecRecord) -> String
where
    T: serde::de::DeserializeOwned + borsh::BorshSerialize,
{
    let typed: T = serde_json::from_value(r.value.clone())
        .unwrap_or_else(|e| panic!("decode {} as typed value: {e}", r.name));
    hex::encode(ducp_types::canonical_bytes(&typed))
}

#[test]
fn signed_transfer_vector_signature_verifies() {
    let r = committed()
        .into_iter()
        .find(|r| r.name == "signed_transfer")
        .expect("signed_transfer vector present");
    let tx: SignedTx = serde_json::from_value(r.value).unwrap();
    assert!(tx.verify_sig(), "published signed tx must verify");
}
