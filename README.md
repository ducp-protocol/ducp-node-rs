# DUCP Node — Rust Reference Implementation

The reference implementation of the **Decentralized Universal Compute Protocol (DUCP)**, in Rust.

> **Status:** scaffold for spec **v0.1.0** — not yet functional. This repository lays out the architecture; the subsystems are stubs.

> **"node" means a network participant** — the software you run to take part in the DUCP network (as a Provider, Validator, etc.). It is **not** Node.js and contains no JavaScript.

## What this is

A node implementation of the DUCP protocol. The protocol itself — the white paper and the normative specification — lives in [`ducp-protocol/spec`](https://github.com/ducp-protocol/spec). This repository implements **spec v0.1.0**.

## Workspace layout

| Crate | Path | Responsibility |
|---|---|---|
| `ducp-dvm` | `crates/dvm` | The DUCP Virtual Machine — deterministic execution and 𝕌 (UCU) metering |
| `ducp-verification` | `crates/verification` | Layered verification: TEE attestation, ZK proofs, sampled re-execution |
| `ducp-ledger` | `crates/ledger` | Settlement of 𝕌 and the Standing reputation ledger |
| `ducp-consensus` | `crates/consensus` | Transaction ordering and finality |
| `ducp-governance` | `crates/governance` | Reputation-weighted, role-chamber governance |
| `ducp-node` | `node` | The binary that wires the subsystems into a runnable node |

## Build & run

```
cargo build
cargo run -p ducp-node
```

Requires a recent stable Rust toolchain (pinned in `rust-toolchain.toml`).

## Specification <-> implementation

This implementation pins to a specific specification version; the current target is **DUCP spec v0.1.0**. Anything that would change the protocol belongs first as a proposal in the [spec repo](https://github.com/ducp-protocol/spec/tree/main/proposals) — this repository implements the spec, it does not define it.

## Contributing

Contributions are welcome under the project's Contributor License Agreement — see [CONTRIBUTING](CONTRIBUTING.md) and the [CLA](https://github.com/ducp-protocol/spec/blob/main/CLA.md). All participation follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Business Source License 1.1 (source-available), converting to Apache-2.0 at v1.0 — see [LICENSE](LICENSE). Draft, pending counsel review.

© 2026 Pawan Singh. "DUCP", "Decentralized Universal Compute Protocol", and the 𝕌 / UCU mark are trademarks of the author.
