# DUCP Profile 0 — Conformance Test Vectors

Golden vectors published with the reference implementation. A conforming node MUST
reproduce all of them (spec/implementation/05 §5, spec/09 §10). They are the
per-milestone acceptance gates and the cross-implementation interop contract.

Loaded by the [`ducp-conformance`](../crates/conformance) harness.

| Family | Dir | Source | Milestone |
|---|---|---|---|
| Codec / hash | `codec/` | spec/implementation/01 §7 | M0 |
| Metering | `metering/` | spec/implementation/02 §5 | M1 |
| Settlement | `settlement/` | spec/implementation/04 §3 | M2/M3 |
| Fraud | `fraud/` | spec/implementation/03 §4 | M4/M5 |
| Replication | `replication/` | spec/implementation/04 §6 | M6 |
| ℚ observable | `q-observable/` | spec/09 §10, DP-0001 §9 | cross-cutting |

Binary values are hex-encoded strings. Amounts are decimal strings (𝕌 base units,
1 𝕌 = 10⁹). All hashes are BLAKE3-256 over canonical (borsh) bytes.
