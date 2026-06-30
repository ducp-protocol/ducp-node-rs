//! Regenerate the committed conformance vector files from the reference crates.
//!
//! Run: `cargo run -p ducp-conformance --bin gen-vectors`
//!
//! Writes the codec/hash family now; later milestones extend this with the
//! metering, settlement, fraud, replication, and ℚ-observable families.

use std::fs;

fn main() {
    let dir = ducp_conformance::vectors_dir();

    // --- codec / hash (M0, spec/bindings/01 §7) ---
    let codec_dir = dir.join("codec");
    fs::create_dir_all(&codec_dir).expect("create codec dir");
    let records = ducp_conformance::codec_records();
    let json = serde_json::to_string_pretty(&records).expect("serialize records");
    let path = codec_dir.join("types.json");
    fs::write(&path, format!("{json}\n")).expect("write codec vectors");
    println!("wrote {} ({} records)", path.display(), records.len());

    // --- metering (M1, spec/bindings/02 §5) ---
    let metering_dir = dir.join("metering");
    fs::create_dir_all(&metering_dir).expect("create metering dir");
    let metering = ducp_conformance::metering_records();
    let json = serde_json::to_string_pretty(&metering).expect("serialize metering");
    let path = metering_dir.join("cases.json");
    fs::write(&path, format!("{json}\n")).expect("write metering vectors");
    println!("wrote {} ({} records)", path.display(), metering.len());

    // --- settlement (M2/M3, spec/bindings/04 §3) ---
    let settlement_dir = dir.join("settlement");
    fs::create_dir_all(&settlement_dir).expect("create settlement dir");
    let settlement = ducp_conformance::settlement_record();
    let json = serde_json::to_string_pretty(&settlement).expect("serialize settlement");
    let path = settlement_dir.join("happy_path.json");
    fs::write(&path, format!("{json}\n")).expect("write settlement vector");
    println!("wrote {}", path.display());

    // --- finality / clawback window (M5, spec/bindings/04 §3) ---
    let finality = ducp_conformance::finality_record();
    let json = serde_json::to_string_pretty(&finality).expect("serialize finality");
    let path = settlement_dir.join("finality.json");
    fs::write(&path, format!("{json}\n")).expect("write finality vector");
    println!("wrote {}", path.display());

    // --- fraud (M4/M5, spec/bindings/03 §4, 04 §4) ---
    let fraud_dir = dir.join("fraud");
    fs::create_dir_all(&fraud_dir).expect("create fraud dir");
    let fraud = ducp_conformance::fraud_record();
    let json = serde_json::to_string_pretty(&fraud).expect("serialize fraud");
    let path = fraud_dir.join("challenge.json");
    fs::write(&path, format!("{json}\n")).expect("write fraud vector");
    println!("wrote {}", path.display());

    // --- replication (M6, spec/bindings/04 §6) ---
    let replication_dir = dir.join("replication");
    fs::create_dir_all(&replication_dir).expect("create replication dir");
    let replication = ducp_conformance::replication_record();
    let json = serde_json::to_string_pretty(&replication).expect("serialize replication");
    let path = replication_dir.join("blocks.json");
    fs::write(&path, format!("{json}\n")).expect("write replication vector");
    println!("wrote {}", path.display());

    // --- ℚ observable (spec/09 §10, DP-0001 §9) ---
    let q_dir = dir.join("q-observable");
    fs::create_dir_all(&q_dir).expect("create q-observable dir");
    let q = ducp_conformance::q_observable_record();
    let json = serde_json::to_string_pretty(&q).expect("serialize q-observable");
    let path = q_dir.join("dp0001.json");
    fs::write(&path, format!("{json}\n")).expect("write q-observable vector");
    println!("wrote {}", path.display());
}
