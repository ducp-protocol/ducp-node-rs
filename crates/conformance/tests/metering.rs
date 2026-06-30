//! M1 conformance: metering golden vectors (spec/bindings/02 §5).
//!
//! Verifies the deterministic 𝕌 derivation: the reference workload meters to
//! exactly one 𝕌, two independent runs agree, and the committed vectors match the
//! reference DVM. `total_fuel` is wasmtime-fuel-model-specific (provisional);
//! regenerate with `gen-vectors` after a metering-semantics change.

use ducp_conformance::{load_json, metering_records, MeteringRecord};
use ducp_dvm::{echo_module, reference_module, Benchmark, Dvm, WasmtimeDvm, REFERENCE_INPUT};
use ducp_types::{Limits, UCU_SCALE};

fn committed() -> Vec<MeteringRecord> {
    load_json("metering", "cases.json")
}

#[test]
fn committed_vectors_match_reference_dvm() {
    assert_eq!(
        committed(),
        metering_records(),
        "metering vectors drifted; regenerate with gen-vectors if the fuel model changed"
    );
}

#[test]
fn reference_workload_is_exactly_one_ucu() {
    let dvm = WasmtimeDvm::new();
    let bench = Benchmark::devnet(&dvm);
    let ucu = dvm.meter(&reference_module(), REFERENCE_INPUT, &bench);
    assert_eq!(ucu, UCU_SCALE);

    let rec = committed();
    let reference = rec.iter().find(|r| r.name == "reference").unwrap();
    assert_eq!(reference.ucu_count, UCU_SCALE.to_string());
}

#[test]
fn two_independent_runs_are_identical() {
    let dvm = WasmtimeDvm::new();
    let bench = Benchmark::devnet(&dvm);
    let limits = Limits {
        max_ucu: 1_000 * UCU_SCALE,
        max_memory_bytes: 16 * 1024 * 1024,
    };
    let a = dvm.execute(&echo_module(), b"hello world", &limits, &bench);
    let b = dvm.execute(&echo_module(), b"hello world", &limits, &bench);
    assert_eq!(a.result_hash, b.result_hash);
    assert_eq!(a.ucu_count, b.ucu_count);
    assert_eq!(a.output, b.output);

    // …and they match the committed result hash.
    let rec = committed();
    let hello = rec.iter().find(|r| r.name == "echo_hello_world").unwrap();
    assert_eq!(hex::encode(a.result_hash), hello.result_hash);
}
