//! # ducp-conformance
//!
//! Reference-node binding conformance harness. Loads the published golden vectors from the
//! workspace-root `test-vectors/` directory and exposes helpers + canonical sample
//! values the per-milestone integration tests (under `tests/`) check the reference
//! crates against.
//!
//! The six vector families (spec/bindings/05 §5, spec/09 §10):
//! `codec`, `metering`, `settlement`, `fraud`, `replication`, `q-observable`.
//!
//! Regenerate the committed vector files with the generator binary:
//! `cargo run -p ducp-conformance --bin gen-vectors`.
//!
//! Specification: <https://github.com/ducp-protocol/ducp-spec>

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Absolute path to the workspace-root `test-vectors/` directory.
pub fn vectors_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR = <workspace>/crates/conformance
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test-vectors")
}

/// Load and parse a JSON vector file under `test-vectors/<family>/<name>`.
pub fn load_json<T: serde::de::DeserializeOwned>(family: &str, name: &str) -> T {
    let path = vectors_dir().join(family).join(name);
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("read vector {}: {e}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("parse vector {}: {e}", path.display()))
}

/// Decode a `0x`-optional hex string into bytes (vectors store binary as hex).
pub fn unhex(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).unwrap_or_else(|e| panic!("bad hex {s:?}: {e}"))
}

/// One codec/hash golden vector (spec/bindings/01 §7): a value, its canonical
/// (borsh) bytes as hex, and the BLAKE3-256 of those bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodecRecord {
    pub name: String,
    pub value: serde_json::Value,
    pub canonical_hex: String,
    pub hash: String,
}

/// Build a [`CodecRecord`] for a value that is both serde- and borsh-encodable.
pub fn codec_record<T>(name: &str, value: &T) -> CodecRecord
where
    T: Serialize + borsh::BorshSerialize,
{
    let canonical = ducp_types::canonical_bytes(value);
    CodecRecord {
        name: name.to_string(),
        value: serde_json::to_value(value).expect("serde value"),
        canonical_hex: hex::encode(&canonical),
        hash: hex::encode(ducp_types::hash_bytes(&canonical)),
    }
}

/// Canonical sample values used to derive the codec golden vectors. Kept in one
/// place so the generator and the test never drift.
pub mod samples {
    use ducp_types::*;

    pub fn task_body() -> TaskBody {
        TaskBody {
            ir: IrId::Wasm,
            program: [0x11; 32],
            input: [0x22; 32],
            limits: Limits {
                max_ucu: 10 * UCU_SCALE,
                max_memory_bytes: 1 << 20,
            },
            tier: VerificationTier::SampledReexec,
            benchmark: 0,
            deadline: 100,
            failure_policy: FailurePolicy::ReturnOnFailure,
            nonce: 7,
        }
    }

    pub fn submission() -> Submission {
        Submission {
            task: task_body().task_id(),
            requester: keys::identity(&[1u8; 32]),
            ucu_count: 0,
            fee: UCU_SCALE / 100,
            status: TaskStatus::Submitted,
            provider: None,
            claim_stake: 0,
        }
    }

    pub fn proof_no_seal() -> ComputeProof {
        ComputeProof {
            task: task_body().task_id(),
            provider: keys::identity(&[2u8; 32]),
            output: content_id(b"result-bytes"),
            result_hash: hash_bytes(b"result-bytes"),
            ucu_count: 4 * UCU_SCALE,
            benchmark: 0,
            tier_data: TierData::SampledReexec,
            power_seal: None,
        }
    }

    pub fn power_seal() -> PowerSeal {
        PowerSeal {
            seal_grade: SealGrade::S1Witnessed,
            boundary: Boundary::Node,
            power_cap_milliwatts: 300_000,
            window_millis: 1_000,
            t_max_millikelvin: 350_000,
            attestation_evidence: content_id(b"attestation-quote"),
            benchmark: 0,
        }
    }

    pub fn proof_with_seal() -> ComputeProof {
        ComputeProof {
            power_seal: Some(power_seal()),
            ..proof_no_seal()
        }
    }

    pub fn receipt() -> Receipt {
        Receipt {
            task: task_body().task_id(),
            paid_to_provider: 4 * UCU_SCALE,
            work_issuance: 4 * UCU_SCALE / 100,
            validator_fee: UCU_SCALE / 100,
            standing_delta: 4 * UCU_SCALE as Sp,
            settled_epoch: 12,
            clawback_until: 44,
        }
    }

    pub fn account() -> Account {
        Account {
            id: keys::identity(&[2u8; 32]),
            balance: 1_000 * UCU_SCALE,
            escrowed: 0,
            bonded: 2 * UCU_SCALE,
        }
    }

    pub fn standing() -> StandingRecord {
        StandingRecord {
            id: keys::identity(&[2u8; 32]),
            sp: 123 * UCU_SCALE as Sp,
            last_decay_epoch: 12,
            strikes: 0,
        }
    }

    pub fn signed_transfer() -> SignedTx {
        SignedTx::sign(
            &[7u8; 32],
            Tx::Transfer {
                to: keys::identity(&[9u8; 32]),
                amount: 5 * UCU_SCALE,
            },
            1,
        )
    }

    pub fn q_entry_null() -> QLedgerEntry {
        QLedgerEntry::unmeasured(task_body().task_id(), 4 * UCU_SCALE, 0)
    }

    pub fn q_entry_valued() -> QLedgerEntry {
        QLedgerEntry {
            task: task_body().task_id(),
            ucu: 4 * UCU_SCALE,
            q: Some(Quant::from_micro(1_640_000)),
            seal_grade: Some(SealGrade::S2Locked),
            boundary: Some(Boundary::Chip),
            benchmark: 0,
        }
    }

    pub fn block() -> Block {
        let tx = signed_transfer();
        Block {
            height: 1,
            parent: [0u8; 32],
            epoch: 0,
            txs: vec![tx.tx_id()],
            state_root: hash_bytes(b"state-root-placeholder"),
            proposer: keys::identity(&[0xAA; 32]),
        }
    }
}

/// One metering golden vector (spec/bindings/02 §5): a canonical module +
/// input and its deterministic `{total_fuel, ucu_count, result_hash}` under the
/// devnet benchmark. `total_fuel` is wasmtime-fuel-model-specific (provisional).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeteringRecord {
    pub name: String,
    pub input_hex: String,
    pub total_fuel: u64,
    pub ucu_count: String,
    pub result_hash: String,
}

/// All metering golden records, freshly computed from the reference DVM + the
/// canonical reference/echo workloads under a calibrated devnet benchmark.
pub fn metering_records() -> Vec<MeteringRecord> {
    use ducp_dvm::{echo_module, reference_module, Benchmark, Dvm, WasmtimeDvm, REFERENCE_INPUT};
    use ducp_types::{Limits, UCU_SCALE};

    let dvm = WasmtimeDvm::new();
    let bench = Benchmark::devnet(&dvm);
    let limits = Limits {
        max_ucu: 1_000 * UCU_SCALE,
        max_memory_bytes: 16 * 1024 * 1024,
    };

    let cases: [(&str, Vec<u8>, Vec<u8>); 3] = [
        ("reference", reference_module(), REFERENCE_INPUT.to_vec()),
        ("echo_empty", echo_module(), Vec::new()),
        ("echo_hello_world", echo_module(), b"hello world".to_vec()),
    ];

    cases
        .into_iter()
        .map(|(name, module, input)| {
            let fuel = dvm
                .measure_fuel(&module, &input)
                .expect("sample workloads complete");
            let outcome = dvm.execute(&module, &input, &limits, &bench);
            MeteringRecord {
                name: name.to_string(),
                input_hex: hex::encode(&input),
                total_fuel: fuel,
                ucu_count: outcome.ucu_count.to_string(),
                result_hash: hex::encode(outcome.result_hash),
            }
        })
        .collect()
}

/// The settlement golden vector (spec/bindings/04 §3): the post-state of a
/// `submit → claim → proof → settle` happy path. Pins the economic outcome, the
/// Receipt, the (𝕌, ℚ) entry (ℚ null in P0), and the `state_root`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettlementRecord {
    pub state_root: String,
    pub requester: ducp_types::Account,
    pub provider: ducp_types::Account,
    pub provider_standing: ducp_types::StandingRecord,
    pub receipt: ducp_types::Receipt,
    pub q_entry: ducp_types::QLedgerEntry,
    pub minted: String,
    pub fee_pool: String,
}

/// The canonical settlement scenario, run against the reference ledger.
pub fn settlement_record() -> SettlementRecord {
    use ducp_governance::Params;
    use ducp_ledger::{apply, State};
    use ducp_types::{
        content_id, keys, ComputeProof, FailurePolicy, IrId, Limits, SignedTx, TaskBody, TierData,
        Tx, VerificationTier, UCU_SCALE,
    };

    let params = Params::devnet();
    let req = keys::identity(&[1u8; 32]);
    let prov = keys::identity(&[2u8; 32]);
    let s = State::genesis(&[(req, 100 * UCU_SCALE), (prov, 100 * UCU_SCALE)], 0);

    let body = TaskBody {
        ir: IrId::Wasm,
        program: content_id(b"program"),
        input: content_id(b"input"),
        limits: Limits {
            max_ucu: 10 * UCU_SCALE,
            max_memory_bytes: 1 << 20,
        },
        tier: VerificationTier::SampledReexec,
        benchmark: 0,
        deadline: 100,
        failure_policy: FailurePolicy::ReturnOnFailure,
        nonce: 1,
    };
    let task = body.task_id();
    let s = apply(
        &s,
        &SignedTx::sign(&[1u8; 32], Tx::SubmitTask(body), 0),
        &params,
    )
    .unwrap();
    let s = apply(
        &s,
        &SignedTx::sign(&[2u8; 32], Tx::ClaimTask { task }, 0),
        &params,
    )
    .unwrap();
    let proof = ComputeProof {
        task,
        provider: prov,
        output: content_id(b"the-output"),
        result_hash: ducp_types::hash_bytes(b"the-output"),
        ucu_count: 4 * UCU_SCALE,
        benchmark: 0,
        tier_data: TierData::SampledReexec,
        power_seal: None,
    };
    let s = apply(
        &s,
        &SignedTx::sign(&[2u8; 32], Tx::SubmitProof(proof), 1),
        &params,
    )
    .unwrap();

    SettlementRecord {
        state_root: hex::encode(s.state_root()),
        requester: s.accounts[&req],
        provider: s.accounts[&prov],
        provider_standing: s.standing[&prov],
        receipt: s.receipts[&task].clone(),
        q_entry: s.q_ledger[&task].clone(),
        minted: s.supply.minted.to_string(),
        fee_pool: s.fee_pool.to_string(),
    }
}

/// The replication golden vector (spec/bindings/04 §6): producing then
/// replaying a sequence of blocks reaches the identical `state_root` — state-machine
/// replication, so the devnet is verifiable even with one proposer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplicationRecord {
    pub blocks: u64,
    pub block_state_roots: Vec<String>,
    pub final_state_root: String,
    pub replica_matches: bool,
}

/// Build a happy-path block sequence and replay it on a fresh replica.
pub fn replication_record() -> ReplicationRecord {
    use ducp_consensus::{ConsensusEngine, SingleSequencer};
    use ducp_governance::Params;
    use ducp_ledger::State;
    use ducp_types::{
        content_id, keys, ComputeProof, FailurePolicy, IrId, Limits, SignedTx, TaskBody, TierData,
        Tx, VerificationTier, UCU_SCALE,
    };

    let params = Params::devnet();
    let proposer = keys::identity(&[0u8; 32]);
    let req = keys::identity(&[1u8; 32]);
    let prov = keys::identity(&[2u8; 32]);
    let genesis = State::genesis(&[(req, 100 * UCU_SCALE), (prov, 100 * UCU_SCALE)], 0);

    let body = TaskBody {
        ir: IrId::Wasm,
        program: content_id(b"program"),
        input: content_id(b"input"),
        limits: Limits {
            max_ucu: 10 * UCU_SCALE,
            max_memory_bytes: 1 << 20,
        },
        tier: VerificationTier::SampledReexec,
        benchmark: 0,
        deadline: 100,
        failure_policy: FailurePolicy::ReturnOnFailure,
        nonce: 1,
    };
    let task = body.task_id();
    let proof = ComputeProof {
        task,
        provider: prov,
        output: content_id(b"out"),
        result_hash: ducp_types::hash_bytes(b"out"),
        ucu_count: 4 * UCU_SCALE,
        benchmark: 0,
        tier_data: TierData::SampledReexec,
        power_seal: None,
    };
    let txs = vec![
        SignedTx::sign(&[1u8; 32], Tx::SubmitTask(body), 0),
        SignedTx::sign(&[2u8; 32], Tx::ClaimTask { task }, 0),
        SignedTx::sign(&[2u8; 32], Tx::SubmitProof(proof), 1),
        SignedTx::sign(
            &[2u8; 32],
            Tx::Transfer {
                to: req,
                amount: UCU_SCALE,
            },
            2,
        ),
    ];

    // Producer: one block per tx.
    let mut prod_seq = SingleSequencer::new(proposer);
    let mut prod_state = genesis.clone();
    let mut blocks = Vec::new();
    for tx in &txs {
        let proposal = prod_seq.produce(std::slice::from_ref(tx), &prod_state, &params);
        if !proposal.block.txs.is_empty() {
            prod_seq.adopt(&proposal.block);
            prod_state = proposal.state.clone();
            blocks.push((proposal.block.clone(), proposal.txs.clone()));
        }
    }
    let final_state_root = prod_state.state_root();

    // Replica: replay the blocks.
    let mut rep_seq = SingleSequencer::new(proposer);
    let mut rep_state = genesis.clone();
    let mut block_state_roots = Vec::new();
    for (block, btxs) in &blocks {
        rep_state = rep_seq
            .commit(block, btxs, &rep_state, &params)
            .expect("replay");
        rep_seq.adopt(block);
        block_state_roots.push(hex::encode(rep_state.state_root()));
    }

    ReplicationRecord {
        blocks: blocks.len() as u64,
        block_state_roots,
        replica_matches: rep_state.state_root() == final_state_root,
        final_state_root: hex::encode(final_state_root),
    }
}

/// The finality golden vector (spec/bindings/04 §3): a settled task whose
/// clawback window closes, releasing the claim stake while the Receipt stays
/// immutable (`I-ECON-FINAL`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinalityRecord {
    pub epoch: u64,
    pub provider: ducp_types::Account,
    pub receipt: ducp_types::Receipt,
    pub released: bool,
    pub receipt_unchanged: bool,
    pub conserved: bool,
    pub state_root: String,
}

/// Build the finality scenario: settle, then advance past the clawback window.
pub fn finality_record() -> FinalityRecord {
    use ducp_governance::Params;
    use ducp_ledger::{advance_to_epoch, apply, State};
    use ducp_types::{
        content_id, keys, ComputeProof, FailurePolicy, IrId, Limits, SignedTx, TaskBody, TierData,
        Tx, VerificationTier, UCU_SCALE,
    };

    let params = Params::devnet();
    let req = keys::identity(&[1u8; 32]);
    let prov = keys::identity(&[2u8; 32]);
    let s = State::genesis(&[(req, 100 * UCU_SCALE), (prov, 100 * UCU_SCALE)], 0);

    let body = TaskBody {
        ir: IrId::Wasm,
        program: content_id(b"program"),
        input: content_id(b"input"),
        limits: Limits {
            max_ucu: 10 * UCU_SCALE,
            max_memory_bytes: 1 << 20,
        },
        tier: VerificationTier::SampledReexec,
        benchmark: 0,
        deadline: 100,
        failure_policy: FailurePolicy::ReturnOnFailure,
        nonce: 1,
    };
    let task = body.task_id();
    let s = apply(
        &s,
        &SignedTx::sign(&[1u8; 32], Tx::SubmitTask(body), 0),
        &params,
    )
    .unwrap();
    let s = apply(
        &s,
        &SignedTx::sign(&[2u8; 32], Tx::ClaimTask { task }, 0),
        &params,
    )
    .unwrap();
    let proof = ComputeProof {
        task,
        provider: prov,
        output: content_id(b"out"),
        result_hash: ducp_types::hash_bytes(b"out"),
        ucu_count: 4 * UCU_SCALE,
        benchmark: 0,
        tier_data: TierData::SampledReexec,
        power_seal: None,
    };
    let s = apply(
        &s,
        &SignedTx::sign(&[2u8; 32], Tx::SubmitProof(proof), 1),
        &params,
    )
    .unwrap();
    let receipt_before = s.receipts[&task].clone();

    let s = advance_to_epoch(&s, params.clawback_epochs, &params);

    FinalityRecord {
        epoch: s.epoch,
        provider: s.accounts[&prov],
        receipt: s.receipts[&task].clone(),
        released: s.released.contains(&task),
        receipt_unchanged: s.receipts[&task] == receipt_before,
        conserved: s.check_conservation(),
        state_root: hex::encode(s.state_root()),
    }
}

/// The fraud golden vector (spec/bindings/03 §4, 04 §4): a forged proof, the
/// re-execution verdict, and the post-resolution state (clawback, burn, fine,
/// Standing floor). Verifies `I-LEDGER-CONSERVE` across the fraud path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FraudRecord {
    pub forged_result_hash: String,
    pub true_result_hash: String,
    pub verdict_is_fraud: bool,
    pub requester: ducp_types::Account,
    pub provider: ducp_types::Account,
    pub challenger: ducp_types::Account,
    pub provider_standing: ducp_types::StandingRecord,
    pub minted: String,
    pub burned: String,
    pub fee_pool: String,
    pub conserved: bool,
    pub state_root: String,
}

/// Build the fraud scenario against the reference crates: settle a forged proof,
/// re-execute it, and resolve the challenge.
pub fn fraud_record() -> FraudRecord {
    use ducp_dvm::{echo_module, Benchmark, Dvm, WasmtimeDvm};
    use ducp_governance::Params;
    use ducp_ledger::{apply, resolve_challenge, State};
    use ducp_types::{
        content_id, keys, ComputeProof, FailurePolicy, IrId, Limits, SignedTx, TaskBody, TierData,
        Tx, VerificationTier, UCU_SCALE,
    };
    use ducp_verification::{SampledReexecVerifier, Verifier};

    let params = Params::devnet();
    let dvm = WasmtimeDvm::new();
    let bench = Benchmark::devnet(&dvm);

    let req = keys::identity(&[1u8; 32]);
    let prov = keys::identity(&[2u8; 32]);
    let chal = keys::identity(&[3u8; 32]);
    let s = State::genesis(
        &[
            (req, 100 * UCU_SCALE),
            (prov, 100 * UCU_SCALE),
            (chal, 100 * UCU_SCALE),
        ],
        0,
    );

    let program = echo_module();
    let input = b"verify".to_vec();
    let body = TaskBody {
        ir: IrId::Wasm,
        program: content_id(&program),
        input: content_id(&input),
        limits: Limits {
            max_ucu: 10 * UCU_SCALE,
            max_memory_bytes: 16 * 1024 * 1024,
        },
        tier: VerificationTier::SampledReexec,
        benchmark: 0,
        deadline: 100,
        failure_policy: FailurePolicy::ReturnOnFailure,
        nonce: 1,
    };
    let task = body.task_id();
    let s = apply(
        &s,
        &SignedTx::sign(&[1u8; 32], Tx::SubmitTask(body.clone()), 0),
        &params,
    )
    .unwrap();
    let s = apply(
        &s,
        &SignedTx::sign(&[2u8; 32], Tx::ClaimTask { task }, 0),
        &params,
    )
    .unwrap();

    let honest = dvm.execute(&program, &input, &body.limits, &bench);
    let forged_result_hash = [0xBAu8; 32];
    let proof = ComputeProof {
        task,
        provider: prov,
        output: content_id(b"forged"),
        result_hash: forged_result_hash,
        ucu_count: honest.ucu_count,
        benchmark: 0,
        tier_data: TierData::SampledReexec,
        power_seal: None,
    };
    let s = apply(
        &s,
        &SignedTx::sign(&[2u8; 32], Tx::SubmitProof(proof.clone()), 1),
        &params,
    )
    .unwrap();

    let bond = params.bond_min(honest.ucu_count).max(1);
    let s = apply(
        &s,
        &SignedTx::sign(&[3u8; 32], Tx::Challenge { task, bond }, 0),
        &params,
    )
    .unwrap();

    // Forced re-execution → verdict.
    let outcome = SampledReexecVerifier.check(&proof, &program, &input, &body.limits, &bench, &dvm);
    let verdict_is_fraud = outcome.is_fraud();

    let s = resolve_challenge(&s, task, verdict_is_fraud, &params);

    FraudRecord {
        forged_result_hash: hex::encode(forged_result_hash),
        true_result_hash: hex::encode(honest.result_hash),
        verdict_is_fraud,
        requester: s.accounts[&req],
        provider: s.accounts[&prov],
        challenger: s.accounts[&chal],
        provider_standing: s.standing[&prov],
        minted: s.supply.minted.to_string(),
        burned: s.supply.burned.to_string(),
        fee_pool: s.fee_pool.to_string(),
        conserved: s.check_conservation(),
        state_root: hex::encode(s.state_root()),
    }
}

/// One row of the ℚ-observable conformance table (spec/09 §10, DP-0001 §9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QProviderRow {
    pub label: String,
    pub micro_q: Option<u64>,
    pub seal_grade: Option<ducp_types::SealGrade>,
    pub boundary: Option<ducp_types::Boundary>,
    pub paid_ucu: String,
}

/// The ℚ-observable golden vector: four Providers run the same task; all are paid an
/// identical 𝕌, recording differing ℚ (and null for the unsealed one) — the
/// reward-neutral (𝕌, ℚ) record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QObservableRecord {
    pub ucu: String,
    pub providers: Vec<QProviderRow>,
    pub all_payments_equal: bool,
}

/// Build the ℚ-observable table from the reference SealedAttestor (spec/09 §10).
pub fn q_observable_record() -> QObservableRecord {
    use ducp_dvm::Benchmark;
    use ducp_types::{content_id, Boundary, PowerSeal, SealGrade, UCU_SCALE};
    use ducp_verification::{EnergyAttestor, SealedAttestor};

    // ℚ baseline: 13.7 pJ/𝕌 at 300 K (provisional integer units: 0.1 pJ; mK).
    let bench = Benchmark {
        version: 0,
        fuel_cost_table_hash: [0u8; 32],
        fuel_per_ucu: 1,
        e_baseline: 137,
        t_std_millikelvin: 300_000,
    };
    let ucu: ducp_types::Ucu = 50_000 * UCU_SCALE;

    let mk_seal = |power_cap: u64, t_max: u64, grade: SealGrade, boundary: Boundary| PowerSeal {
        seal_grade: grade,
        boundary,
        power_cap_milliwatts: power_cap, // per-𝕌 energy (window = C)
        window_millis: 50_000,
        t_max_millikelvin: t_max,
        attestation_evidence: content_id(b"root-of-trust-quote"),
        benchmark: 0,
    };

    let sealed = [
        (
            "A",
            Some((274u64, 350_000u64, SealGrade::S1Witnessed, Boundary::Node)),
        ),
        (
            "B",
            Some((137, 300_000, SealGrade::S2Locked, Boundary::Chip)),
        ),
        (
            "C",
            Some((100, 250_000, SealGrade::S2Locked, Boundary::Chip)),
        ),
        ("D", None),
    ];

    let providers: Vec<QProviderRow> = sealed
        .into_iter()
        .map(|(label, params)| {
            let (micro_q, grade, boundary) = match params {
                Some((cap, t, g, b)) => {
                    let q = SealedAttestor
                        .attest(&mk_seal(cap, t, g, b), ucu, &bench)
                        .map(|q| q.micro_q);
                    (q, Some(g), Some(b))
                }
                None => (None, None, None),
            };
            QProviderRow {
                label: label.to_string(),
                micro_q,
                seal_grade: grade,
                boundary,
                // Base payment is the metered 𝕌 — identical for all (reward-neutral).
                paid_ucu: ucu.to_string(),
            }
        })
        .collect();

    let all_payments_equal = providers.iter().all(|p| p.paid_ucu == ucu.to_string());

    QObservableRecord {
        ucu: ucu.to_string(),
        providers,
        all_payments_equal,
    }
}

/// All codec golden records, freshly computed from [`samples`]. The committed file
/// `test-vectors/codec/types.json` MUST equal this (it is generated from it).
pub fn codec_records() -> Vec<CodecRecord> {
    use samples as s;
    vec![
        codec_record("task_body", &s::task_body()),
        codec_record("limits", &s::task_body().limits),
        codec_record("submission", &s::submission()),
        codec_record("compute_proof_no_seal", &s::proof_no_seal()),
        codec_record("power_seal", &s::power_seal()),
        codec_record("compute_proof_with_seal", &s::proof_with_seal()),
        codec_record("receipt", &s::receipt()),
        codec_record("account", &s::account()),
        codec_record("standing_record", &s::standing()),
        codec_record("signed_transfer", &s::signed_transfer()),
        codec_record("q_entry_null", &s::q_entry_null()),
        codec_record("q_entry_valued", &s::q_entry_valued()),
        codec_record("block", &s::block()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_dir_exists() {
        let d = vectors_dir();
        assert!(d.is_dir(), "test-vectors dir missing at {}", d.display());
    }

    #[test]
    fn unhex_roundtrip() {
        assert_eq!(unhex("0x00ff"), vec![0u8, 255]);
        assert_eq!(unhex("00ff"), vec![0u8, 255]);
    }

    #[test]
    fn all_six_vector_families_present() {
        // spec/bindings/05 §5 (five) + spec/09 §10 (ℚ).
        let families = [
            ("codec", "types.json"),
            ("metering", "cases.json"),
            ("settlement", "happy_path.json"),
            ("fraud", "challenge.json"),
            ("replication", "blocks.json"),
            ("q-observable", "dp0001.json"),
        ];
        for (family, file) in families {
            let path = vectors_dir().join(family).join(file);
            assert!(path.is_file(), "missing vector {}", path.display());
        }
    }
}
