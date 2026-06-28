//! DUCP devnet worker — a beachhead load driver.
//!
//! Connects to a running sequencer over JSON-RPC and repeatedly runs the full task
//! lifecycle (submit → claim → execute → proof → settle) against a deterministic
//! WebAssembly workload, printing the settled 𝕌 and the (𝕌, ℚ) record for each.
//!
//! Acts as both Requester and Provider for the demo, using two dev keys that the
//! default `ducp-node` genesis funds (seeds `01..` and `02..`).

use clap::Parser;
use ducp_dvm::{echo_module, Benchmark, Dvm, WasmtimeDvm};
use ducp_node::DucpApiClient;
use ducp_types::{
    content_id, keys, ComputeProof, FailurePolicy, IrId, Limits, SignedTx, TaskBody, TierData, Tx,
    VerificationTier, UCU_SCALE,
};
use jsonrpsee::http_client::HttpClientBuilder;

#[derive(Parser, Debug)]
#[command(name = "ducp-worker", version, about)]
struct Cli {
    /// Sequencer JSON-RPC URL.
    #[arg(long, default_value = "http://127.0.0.1:8645")]
    sequencer: String,

    /// Number of beachhead tasks to run.
    #[arg(long, default_value_t = 5)]
    tasks: u64,

    /// Hex seed for the Requester key (funded at genesis).
    #[arg(
        long,
        default_value = "0101010101010101010101010101010101010101010101010101010101010101"
    )]
    requester_seed: String,

    /// Hex seed for the Provider key (funded at genesis).
    #[arg(
        long,
        default_value = "0202020202020202020202020202020202020202020202020202020202020202"
    )]
    provider_seed: String,
}

fn seed(hex_str: &str) -> [u8; 32] {
    hex::decode(hex_str).unwrap().try_into().unwrap()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let req_seed = seed(&cli.requester_seed);
    let prov_seed = seed(&cli.provider_seed);
    let prov = keys::identity(&prov_seed);

    let client = HttpClientBuilder::default().build(&cli.sequencer)?;
    let dvm = WasmtimeDvm::new();
    let bench = Benchmark::devnet(&dvm);

    // Upload the program once.
    let program = echo_module();
    let prog_id = client.put_blob(hex::encode(&program)).await?.content_id;

    let head = client.get_head().await?;
    println!(
        "connected to {} — head height {} epoch {}",
        cli.sequencer, head.height, head.epoch
    );

    // Local nonces (fresh devnet starts at 0). The requester submits once per task,
    // so its nonce is the task index `i`; the provider sends two txs per task.
    let mut prov_nonce = 0u64;

    let mut total_ucu: u128 = 0;
    for i in 0..cli.tasks {
        let input = format!("beachhead-task-{i}").into_bytes();
        let in_id = client.put_blob(hex::encode(&input)).await?.content_id;

        let body = TaskBody {
            ir: IrId::Wasm,
            program: hex::decode(&prog_id)?.try_into().unwrap(),
            input: hex::decode(&in_id)?.try_into().unwrap(),
            limits: Limits {
                max_ucu: 10 * UCU_SCALE,
                max_memory_bytes: 16 * 1024 * 1024,
            },
            tier: VerificationTier::SampledReexec,
            benchmark: 0,
            deadline: u64::MAX,
            failure_policy: FailurePolicy::ReturnOnFailure,
            nonce: i,
        };
        let task = body.task_id();

        client
            .submit_task(SignedTx::sign(&req_seed, Tx::SubmitTask(body.clone()), i))
            .await?;

        client
            .claim_task(SignedTx::sign(
                &prov_seed,
                Tx::ClaimTask { task },
                prov_nonce,
            ))
            .await?;
        prov_nonce += 1;

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
            .submit_proof(SignedTx::sign(
                &prov_seed,
                Tx::SubmitProof(proof),
                prov_nonce,
            ))
            .await?;
        prov_nonce += 1;

        let q = client.get_q_entry(hex::encode(task)).await?;
        let q_str = match q.q {
            Some(v) => format!("{}.{:06}", v.micro_q / 1_000_000, v.micro_q % 1_000_000),
            None => "null".to_string(),
        };
        total_ucu += outcome.ucu_count;
        println!(
            "task {i}: settled 𝕌={} ℚ={} (task {})",
            outcome.ucu_count,
            q_str,
            &hex::encode(task)[..16]
        );
    }

    let head = client.get_head().await?;
    println!(
        "done — {} tasks, total 𝕌={}, head height {} state_root {}",
        cli.tasks,
        total_ucu,
        head.height,
        &head.state_root[..16]
    );
    Ok(())
}
