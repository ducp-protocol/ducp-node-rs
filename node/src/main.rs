//! DUCP reference node (Rust).
//!
//! Runs a node on the DUCP network. Here, "node" means a *network participant* —
//! the software you run to take part in DUCP (as a Provider, Validator, and so
//! on). It is unrelated to Node.js and contains no JavaScript.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//! Status: scaffold for spec v0.1.0 — not yet operational.

fn main() {
    println!(
        "DUCP node (Rust reference implementation) v{}",
        env!("CARGO_PKG_VERSION")
    );
    println!("  implements DUCP specification v0.1.0");
    println!("  https://github.com/ducp-protocol/spec");
    println!();
    println!("subsystems (scaffold — not yet implemented):");
    println!("  dvm           v{}", ducp_dvm::version());
    println!("  verification  v{}", ducp_verification::version());
    println!("  ledger        v{}", ducp_ledger::version());
    println!("  consensus     v{}", ducp_consensus::version());
    println!("  governance    v{}", ducp_governance::version());
}
