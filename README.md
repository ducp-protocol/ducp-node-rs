# DUCP Node — Rust Reference Implementation

The reference implementation of the **Decentralized Universal Compute Protocol (DUCP)**, in Rust.

> **Status:** **Reference implementation** for DUCP-SPEC **v0.2.0** (reference-node binding, MVP / devnet) — functional end to end. A single-sequencer devnet accepts a task, executes it deterministically, verifies it by sampled re-execution / open challenge, and settles it with real 𝕌 and Standing accounting. Advanced tiers (TEE/ZK, BFT, trustless energy attestation, on-chain governance) sit behind traits as documented seams.

> **"node" means a network participant** — the software you run to take part in the DUCP network (as a Provider, Validator, etc.). It is **not** Node.js and contains no JavaScript.

## What this is

A node implementation of the DUCP protocol. The protocol itself — the white paper, normative specification, and reference-node binding — lives in [`ducp-protocol/spec`](https://github.com/ducp-protocol/spec). This repository is the **reference implementation** for **DUCP-SPEC v0.2.0**, conforming to the pinned choices in [`spec/bindings/`](https://github.com/ducp-protocol/spec/tree/main/spec/bindings).

The reference-node binding pins the buildable choices — **WebAssembly** IR (wasmtime), **single-sequencer** devnet, **sampled re-execution** — while preserving every protocol invariant. The Quant (ℚ) efficiency observable is recorded as the reward-neutral **(𝕌, ℚ)** pair from genesis (ℚ null until energy attestation exists).

## Workspace layout

This repository is a **Cargo workspace**: reusable protocol crates live under `crates/`, and the runnable node is a separate member at `ducp-node/`. That split is deliberate — libraries stay small, dependency boundaries stay explicit, and each crate can be tested and versioned on its own — while the root `Cargo.toml` stays a thin workspace manifest (not a monolithic `src/` tree). Putting the binary crate in `ducp-node/` (rather than at the repo root) keeps that layout symmetric and leaves room for additional binaries without cluttering the workspace root.

| Crate | Path | Responsibility |
|---|---|---|
| `ducp-types` | `crates/ducp-types` | Canonical data model: identifiers, tasks, records, txs/blocks, the Compute Proof, and the ℚ types; borsh codec, BLAKE3 hashing, Ed25519 keys |
| `ducp-dvm` | `crates/dvm` | The DUCP Virtual Machine — deterministic WebAssembly execution and fuel-based 𝕌 metering; the benchmark; the `Ir` registry seam |
| `ducp-verification` | `crates/verification` | Sampled re-execution + challenge; the `EnergyAttestor` seam (ℚ); reserved TEE/ZK verifier seams |
| `ducp-ledger` | `crates/ledger` | The state machine: accounts, 𝕌, Standing, the ℚ-ledger, settlement, fraud resolution, clawback/finality |
| `ducp-consensus` | `crates/consensus` | Transaction ordering and finality — `SingleSequencer` (BFT is a later `ConsensusEngine`) |
| `ducp-governance` | `crates/governance` | Static parameter set (devnet defaults); the `ParamSource` on-chain-governance seam |
| `ducp-node` | `ducp-node` | The node binary + JSON-RPC server, mempool, scheduler, keystore, blob store; plus the `ducp-worker` load driver |
| `ducp-conformance` | `crates/conformance` | Loads the published golden vectors and checks them against the reference crates |

## Build & run

```bash
cargo build
cargo test --workspace        # unit + integration + conformance

# Run a single-sequencer node:
cargo run -p ducp-node -- --listen 127.0.0.1:8645
```

Requires a recent stable Rust toolchain (pinned in `rust-toolchain.toml`); the DVM uses [wasmtime](https://wasmtime.dev/).

### Devnet demo (1 sequencer + 1 worker)

```bash
PORT=8650 TASKS=5 scripts/devnet.sh
```

This starts a sequencer and runs the beachhead workload against it over JSON-RPC — `submit → claim → execute → proof → settle`, repeatedly — printing the settled 𝕌 and the (𝕌, ℚ) record for each task.

## Architecture & milestones

The node was built along the reference-node binding roadmap (`spec/bindings/README.md`):

| # | Milestone | What it delivers |
|---|---|---|
| M0 | Data model + codecs | `ducp-types`: canonical borsh encoding, BLAKE3 hashing, Ed25519, the ℚ types |
| M1 | Wasm DVM + metering | deterministic wasmtime runtime, fuel → 𝕌, the benchmark, deterministic failures |
| M2 | Devnet ledger | accounts, escrow, settlement, Standing, the ℚ-ledger; `I-LEDGER-CONSERVE` |
| M3 | End-to-end | single-sequencer consensus + JSON-RPC node; `submit → settle` over RPC |
| M4 | Verification | sampled re-execution + open challenge; clawback, offsetting burn, fine, Standing floor |
| M5 | Clawback + finality | bonded stake locked then released; settled tx never rewritten (`I-ECON-FINAL`) |
| M6 | Devnet + dogfood | multi-process devnet, state-machine replication, beachhead workload |

## The Quant (ℚ) efficiency observable

Every settled task records the reward-neutral **(𝕌, ℚ)** pair (DP-0001, spec/09). On the live devnet ℚ is **null** (`NullAttestor`, no energy measured) and the efficiency multiplier on Standing is `1.0`, so base settlement is strictly 𝕌-proportional (`I-Q-REWARDNEUTRAL`). The **Sealed-ℚ floor** computation and the three gated Power-Seal checks are implemented (`SealedAttestor`) and verified against the DP-0001 §9 / spec/09 §10 conformance vector — four providers, identical 𝕌 and payment, ℚ ≈ {0.43, 1.00, 1.64, null}.

## Conformance test vectors

Published in [`test-vectors/`](test-vectors/) and checked by `ducp-conformance`. Six families:

| Family | Source | Milestone |
|---|---|---|
| `codec/` | spec/bindings/01 §7 | M0 |
| `metering/` | spec/bindings/02 §5 | M1 |
| `settlement/` | spec/bindings/04 §3 | M2/M3/M5 |
| `fraud/` | spec/bindings/03 §4, 04 §4 | M4/M5 |
| `replication/` | spec/bindings/04 §6 | M6 |
| `q-observable/` | spec/09 §10, DP-0001 §9 | ℚ |

Regenerate after an intentional change:

```bash
cargo run -p ducp-conformance --bin gen-vectors
```

## Reference-node binding scope & deferred seams

Out of scope for this binding, each represented by a trait so it is additive later: TEE/ZK tiers (`Verifier`), BFT consensus (`ConsensusEngine`), trustless energy attestation and the efficiency bonus (`EnergyAttestor`), multiple IRs (`IrRegistry`), on-chain governance (`ParamSource`), and persistent/Merkle state (`Storage`). Provisional choices (borsh, BLAKE3, Ed25519, `UCU_DECIMALS = 9`, the fuel cost model) are tuned on devnet and frozen toward 1.0.

## Specification <-> implementation

This implementation pins to a specification version; the current target is **DUCP spec v0.2.0**. Anything that would change the protocol belongs first as a proposal in the [spec repo](https://github.com/ducp-protocol/spec/tree/main/proposals) — this repository implements the spec, it does not define it.

## Contributing

Contributions are welcome under the project's Contributor License Agreement — see [CONTRIBUTING](CONTRIBUTING.md) and the [CLA](https://github.com/ducp-protocol/spec/blob/main/CLA.md). All participation follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Business Source License 1.1 (source-available), converting to Apache-2.0 at v1.0 — see [LICENSE](LICENSE). Draft, pending counsel review.

© 2026 Pawan Singh. "DUCP", "Decentralized Universal Compute Protocol", and the 𝕌 / UCU mark are trademarks of the author.
