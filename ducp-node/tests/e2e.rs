//! M3 end-to-end: submit → match → execute → verify(sampling off) → settle, driven
//! over JSON-RPC against a live single-sequencer node. Asserts payment transfer,
//! work-issuance mint, Standing accrual, and the reward-neutral (𝕌, ℚ) entry.

use ducp_consensus::{ConsensusEngine, SingleSequencer};
use ducp_dvm::{echo_module, Benchmark, Dvm, WasmtimeDvm};
use ducp_ledger::State;
use ducp_node::{start_server, DucpApiClient, NodeHandle};
use ducp_types::{
    content_id, keys, ComputeProof, FailurePolicy, IrId, Limits, SignedTx, TaskBody, TierData, Tx,
    VerificationTier, UCU_SCALE,
};
use jsonrpsee::http_client::HttpClientBuilder;

fn h32(s: &str) -> [u8; 32] {
    hex::decode(s).unwrap().try_into().unwrap()
}

#[tokio::test]
async fn happy_path_submit_to_settle_over_rpc() {
    let req_seed = [1u8; 32];
    let prov_seed = [2u8; 32];
    let req = keys::identity(&req_seed);
    let prov = keys::identity(&prov_seed);

    let handle = NodeHandle::new(
        keys::identity(&[0u8; 32]),
        &[(req, 1_000 * UCU_SCALE), (prov, 1_000 * UCU_SCALE)],
    );
    let (addr, _server) = start_server(handle, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let client = HttpClientBuilder::default()
        .build(format!("http://{addr}"))
        .unwrap();

    // Requester uploads program + input as content-addressed blobs.
    let program = echo_module();
    let input = b"hello ducp".to_vec();
    let prog_id = client
        .put_blob(hex::encode(&program))
        .await
        .unwrap()
        .content_id;
    let in_id = client
        .put_blob(hex::encode(&input))
        .await
        .unwrap()
        .content_id;

    // Advisory metering before committing.
    let est = client
        .estimate_ucu(prog_id.clone(), in_id.clone(), 0)
        .await
        .unwrap();
    assert_ne!(est.ucu, "0");

    // Submit the task.
    let body = TaskBody {
        ir: IrId::Wasm,
        program: h32(&prog_id),
        input: h32(&in_id),
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
    let submit = client
        .submit_task(SignedTx::sign(&req_seed, Tx::SubmitTask(body.clone()), 0))
        .await
        .unwrap();
    assert_eq!(submit.task_id, hex::encode(task));

    // Provider claims (posts Standing-discounted stake).
    let claim = client
        .claim_task(SignedTx::sign(&prov_seed, Tx::ClaimTask { task }, 0))
        .await
        .unwrap();
    assert!(claim.ok);
    assert_eq!(claim.claim_stake, (5 * UCU_SCALE).to_string()); // 0.5 · max_ucu

    // Provider executes deterministically and submits the proof.
    let dvm = WasmtimeDvm::new();
    let bench = Benchmark::devnet(&dvm);
    let outcome = dvm.execute(&program, &input, &body.limits, &bench);
    assert_eq!(outcome.output, input); // echo
    let proof = ComputeProof {
        task,
        provider: prov,
        output: content_id(&outcome.output),
        result_hash: outcome.result_hash,
        ucu_count: outcome.ucu_count,
        benchmark: 0,
        tier_data: TierData::SampledReexec,
        power_seal: None,
    };
    let ok = client
        .submit_proof(SignedTx::sign(&prov_seed, Tx::SubmitProof(proof), 1))
        .await
        .unwrap();
    assert!(ok.ok);

    // Task settled.
    let tv = client.get_task(hex::encode(task)).await.unwrap();
    assert_eq!(tv.status, "Settled");
    let receipt = tv.receipt.expect("receipt recorded");
    assert_eq!(receipt.paid_to_provider, outcome.ucu_count);

    // (𝕌, ℚ) pair recorded, ℚ null (I-Q-NULL / reward-neutral).
    let q = client.get_q_entry(hex::encode(task)).await.unwrap();
    assert!(q.q.is_none());
    assert_eq!(q.ucu, outcome.ucu_count);

    // Provider: stake bonded; Standing accrued 1:1 with 𝕌.
    let acct = client.get_account(hex::encode(prov)).await.unwrap();
    assert_eq!(acct.bonded, (5 * UCU_SCALE).to_string());
    let st = client.get_standing(hex::encode(prov)).await.unwrap();
    assert_eq!(st.sp, outcome.ucu_count.to_string());

    // Head advanced past the three blocks (submit, claim, proof+settle).
    let head = client.get_head().await.unwrap();
    assert!(head.height >= 3, "head height {}", head.height);
}

#[tokio::test]
async fn fraudulent_proof_is_challenged_and_slashed() {
    let req_seed = [1u8; 32];
    let prov_seed = [2u8; 32];
    let chal_seed = [3u8; 32];
    let req = keys::identity(&req_seed);
    let prov = keys::identity(&prov_seed);
    let chal = keys::identity(&chal_seed);

    let handle = NodeHandle::new(
        keys::identity(&[0u8; 32]),
        &[
            (req, 1_000 * UCU_SCALE),
            (prov, 1_000 * UCU_SCALE),
            (chal, 1_000 * UCU_SCALE),
        ],
    );
    let (addr, _server) = start_server(handle, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let client = HttpClientBuilder::default()
        .build(format!("http://{addr}"))
        .unwrap();

    let program = echo_module();
    let input = b"verify me".to_vec();
    let prog_id = client
        .put_blob(hex::encode(&program))
        .await
        .unwrap()
        .content_id;
    let in_id = client
        .put_blob(hex::encode(&input))
        .await
        .unwrap()
        .content_id;

    let body = TaskBody {
        ir: IrId::Wasm,
        program: h32(&prog_id),
        input: h32(&in_id),
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
    client
        .submit_task(SignedTx::sign(&req_seed, Tx::SubmitTask(body.clone()), 0))
        .await
        .unwrap();
    client
        .claim_task(SignedTx::sign(&prov_seed, Tx::ClaimTask { task }, 0))
        .await
        .unwrap();

    // Provider submits a FRAUDULENT proof: correct ucu_count, forged result_hash.
    let dvm = WasmtimeDvm::new();
    let bench = Benchmark::devnet(&dvm);
    let honest = dvm.execute(&program, &input, &body.limits, &bench);
    let fraud_proof = ComputeProof {
        task,
        provider: prov,
        output: content_id(b"forged"),
        result_hash: [0xBA; 32], // forged — not the real echo hash
        ucu_count: honest.ucu_count,
        benchmark: 0,
        tier_data: TierData::SampledReexec,
        power_seal: None,
    };
    client
        .submit_proof(SignedTx::sign(&prov_seed, Tx::SubmitProof(fraud_proof), 1))
        .await
        .unwrap();

    // It settled optimistically.
    let tv = client.get_task(hex::encode(task)).await.unwrap();
    assert_eq!(tv.status, "Settled");

    // A challenger contests within the clawback window.
    let p = honest.ucu_count;
    let bond = p / 4 + 1; // ≥ bond_min = 0.25·P
    client
        .challenge(SignedTx::sign(&chal_seed, Tx::Challenge { task, bond }, 0))
        .await
        .unwrap();

    // Fraud proven: provider Standing floored, task failed, challenger rewarded.
    let prov_standing = client.get_standing(hex::encode(prov)).await.unwrap();
    assert_eq!(prov_standing.sp, "0");
    assert_eq!(prov_standing.strikes, 1);
    let tv = client.get_task(hex::encode(task)).await.unwrap();
    assert_eq!(tv.status, "Failed");
    let chal_acct = client.get_account(hex::encode(chal)).await.unwrap();
    // Challenger's bonded is released; balance is back near the start (reward ≥ 0).
    assert_eq!(chal_acct.bonded, "0");
}

#[tokio::test]
async fn replica_replays_blocks_to_identical_state_root() {
    // 1 sequencer + a replica that fetches blocks over RPC and replays them.
    let req_seed = [1u8; 32];
    let prov_seed = [2u8; 32];
    let req = keys::identity(&req_seed);
    let prov = keys::identity(&prov_seed);
    let proposer = keys::identity(&[0u8; 32]);
    let allocations = [(req, 1_000 * UCU_SCALE), (prov, 1_000 * UCU_SCALE)];

    let handle = NodeHandle::new(proposer, &allocations);
    let (addr, _server) = start_server(handle, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let client = HttpClientBuilder::default()
        .build(format!("http://{addr}"))
        .unwrap();

    // Drive a happy path so the chain has several blocks.
    let program = echo_module();
    let input = b"replicate me".to_vec();
    let prog_id = client
        .put_blob(hex::encode(&program))
        .await
        .unwrap()
        .content_id;
    let in_id = client
        .put_blob(hex::encode(&input))
        .await
        .unwrap()
        .content_id;
    let body = TaskBody {
        ir: IrId::Wasm,
        program: h32(&prog_id),
        input: h32(&in_id),
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
    client
        .submit_task(SignedTx::sign(&req_seed, Tx::SubmitTask(body.clone()), 0))
        .await
        .unwrap();
    client
        .claim_task(SignedTx::sign(&prov_seed, Tx::ClaimTask { task }, 0))
        .await
        .unwrap();
    let dvm = WasmtimeDvm::new();
    let bench = Benchmark::devnet(&dvm);
    let outcome = dvm.execute(&program, &input, &body.limits, &bench);
    let proof = ComputeProof {
        task,
        provider: prov,
        output: content_id(&outcome.output),
        result_hash: outcome.result_hash,
        ucu_count: outcome.ucu_count,
        benchmark: 0,
        tier_data: TierData::SampledReexec,
        power_seal: None,
    };
    client
        .submit_proof(SignedTx::sign(&prov_seed, Tx::SubmitProof(proof), 1))
        .await
        .unwrap();

    // Replica: fresh genesis + sequencer, replay every block fetched over RPC.
    let head = client.get_head().await.unwrap();
    let mut replica_seq = SingleSequencer::new(proposer);
    let mut replica_state = State::genesis(&allocations, 0);
    for height in 1..=head.height {
        let block = client.get_block(height).await.unwrap();
        let txs = client.get_block_txs(height).await.unwrap();
        replica_state = replica_seq
            .commit(&block, &txs, &replica_state, &Default::default())
            .expect("replica replay");
        replica_seq.adopt(&block);
    }

    // The replica converges on the sequencer's committed state_root.
    assert_eq!(hex::encode(replica_state.state_root()), head.state_root);
}

#[tokio::test]
async fn unknown_task_query_errors() {
    let handle = NodeHandle::new(keys::identity(&[0u8; 32]), &[]);
    let (addr, _server) = start_server(handle, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let client = HttpClientBuilder::default()
        .build(format!("http://{addr}"))
        .unwrap();
    let res = client.get_task(hex::encode([9u8; 32])).await;
    assert!(res.is_err());
}
