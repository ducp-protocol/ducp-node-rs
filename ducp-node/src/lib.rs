//! # ducp_node
//!
//! Reference node: wires the DVM, ledger, single-sequencer consensus, and
//! governance parameters behind a JSON-RPC server (spec/bindings/05).
//!
//! The node accepts signed transactions, orders them into single-sequencer blocks,
//! applies the deterministic ledger transition, and exposes read queries — including
//! the **(𝕌, ℚ)** pair recorded for every settled task (spec/09 §8).
//!
//! "node" means a *network participant*; it is unrelated to Node.js.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//! Status: Reference implementation for DUCP-SPEC v0.2.0.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::server::{Server, ServerHandle};
use jsonrpsee::types::ErrorObjectOwned;
use serde::{Deserialize, Serialize};

use ducp_consensus::{ConsensusEngine, SingleSequencer};
use ducp_dvm::{Benchmark, Dvm, WasmtimeDvm};
use ducp_governance::Params;
use ducp_ledger::{resolve_challenge, State};
use ducp_types::{
    content_id, Account, Block, ContentId, Hash, Identity, QLedgerEntry, Receipt, Reject, SignedTx,
    Submission, TaskId, Tx, TxId, Ucu,
};
use ducp_verification::{SampledReexecVerifier, Verifier};

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ============================ Node state ===================================

/// Persistence seam for ledger state (spec/bindings/04: the state commitment
/// scheme is provisional). The reference-node binding uses [`InMemoryStorage`]; a disk/Merkle-backed
/// store is a later `impl Storage` with no change to the node.
pub trait Storage: Send + Sync {
    /// Persist the latest committed state snapshot.
    fn save(&self, state: &State);
    /// Load the most recent snapshot, if any.
    fn load(&self) -> Option<State>;
}

/// The binding in-memory state store (no durability).
#[derive(Default)]
pub struct InMemoryStorage {
    last: Mutex<Option<State>>,
}

impl Storage for InMemoryStorage {
    fn save(&self, state: &State) {
        *self.last.lock().expect("storage mutex") = Some(state.clone());
    }
    fn load(&self) -> Option<State> {
        self.last.lock().expect("storage mutex").clone()
    }
}

/// Mutable node state guarded by a single mutex.
struct NodeInner {
    state: State,
    sequencer: SingleSequencer,
    mempool: Vec<SignedTx>,
    blobs: BTreeMap<ContentId, Vec<u8>>,
    blocks: Vec<Block>,
    block_txs: Vec<Vec<SignedTx>>,
}

/// A running node: immutable config + engines, plus the mutex-guarded ledger state.
pub struct NodeHandle {
    pub params: Params,
    pub benchmark: Benchmark,
    pub dvm: WasmtimeDvm,
    storage: Box<dyn Storage>,
    inner: Mutex<NodeInner>,
}

impl NodeHandle {
    /// Build a node with the given sequencer identity and genesis 𝕌 allocations,
    /// backed by in-memory storage.
    pub fn new(proposer: Identity, allocations: &[(Identity, Ucu)]) -> Arc<NodeHandle> {
        Self::with_storage(proposer, allocations, Box::<InMemoryStorage>::default())
    }

    /// Build a node with a custom [`Storage`] backend.
    pub fn with_storage(
        proposer: Identity,
        allocations: &[(Identity, Ucu)],
        storage: Box<dyn Storage>,
    ) -> Arc<NodeHandle> {
        let dvm = WasmtimeDvm::new();
        let benchmark = Benchmark::devnet(&dvm);
        let state = State::genesis(allocations, 0);
        storage.save(&state);
        let sequencer = SingleSequencer::new(proposer);
        Arc::new(NodeHandle {
            params: Params::devnet(),
            benchmark,
            dvm,
            storage,
            inner: Mutex::new(NodeInner {
                state,
                sequencer,
                mempool: Vec::new(),
                blobs: BTreeMap::new(),
                blocks: Vec::new(),
                block_txs: Vec::new(),
            }),
        })
    }

    /// Admit a signed transaction and produce a single-sequencer block immediately
    /// (binding devnet finality). Returns the `TxId` on acceptance, or the ledger
    /// `Reject` reason. The block carries any settlement, mint, Standing update, and
    /// the (𝕌, ℚ) entry that the transition produced.
    pub fn submit(&self, stx: SignedTx) -> Result<TxId, Reject> {
        let txid = stx.tx_id();
        let mut inner = self.inner.lock().expect("node mutex");
        inner.mempool.push(stx);
        let mempool = std::mem::take(&mut inner.mempool);
        let proposal = inner
            .sequencer
            .produce(&mempool, &inner.state, &self.params);

        let outcome = proposal.results.iter().find(|r| r.tx_id == txid).cloned();

        if !proposal.block.txs.is_empty() {
            inner.sequencer.adopt(&proposal.block);
            inner.state = proposal.state;
            inner.blocks.push(proposal.block);
            inner.block_txs.push(proposal.txs);
            self.storage.save(&inner.state);
        }

        match outcome {
            Some(r) if r.accepted => Ok(txid),
            Some(r) => Err(r.reject.unwrap_or(Reject::Invalid)),
            None => Err(Reject::Invalid),
        }
    }

    /// Resolve an open challenge against `task` by re-executing the proof and
    /// applying the verdict on-chain (spec/bindings/03 §3). Returns whether
    /// fraud was found. The bond must already be locked (via a Challenge tx). The
    /// resolution is committed as a system block so the head and `state_root`
    /// advance.
    pub fn resolve_pending_challenge(&self, task: TaskId) -> Result<bool, Reject> {
        // Gather re-execution inputs (brief lock).
        let (body, proof, program, input) = {
            let inner = self.inner.lock().expect("node mutex");
            let body = inner
                .state
                .bodies
                .get(&task)
                .ok_or(Reject::UnknownTask)?
                .clone();
            let proof = inner
                .state
                .proofs
                .get(&task)
                .ok_or(Reject::UnknownTask)?
                .clone();
            let program = inner.blobs.get(&body.program).cloned();
            let input = inner.blobs.get(&body.input).cloned();
            (body, proof, program, input)
        };
        let program = program.ok_or(Reject::UnknownTask)?;
        let input = input.ok_or(Reject::UnknownTask)?;

        // Re-execute outside the lock (CPU-bound, deterministic).
        let outcome = SampledReexecVerifier.check(
            &proof,
            &program,
            &input,
            &body.limits,
            &self.benchmark,
            &self.dvm,
        );
        let fraud = outcome.is_fraud();

        // Apply the verdict and seal it as a system block.
        let mut guard = self.inner.lock().expect("node mutex");
        let inner = &mut *guard;
        let resolved = resolve_challenge(&inner.state, task, fraud, &self.params);
        let block = inner.sequencer.seal_block(&resolved);
        inner.sequencer.adopt(&block);
        inner.state = resolved;
        inner.blocks.push(block);
        inner.block_txs.push(Vec::new());
        self.storage.save(&inner.state);
        Ok(fraud)
    }

    /// Advance the epoch boundary (Standing decay + clawback bond release), sealing
    /// the result as a system block.
    pub fn advance_epoch(&self) {
        let mut guard = self.inner.lock().expect("node mutex");
        let inner = &mut *guard; // split disjoint fields past the MutexGuard Deref
        let new_state = inner.sequencer.advance_epoch(&inner.state, &self.params);
        let block = inner.sequencer.seal_block(&new_state);
        inner.sequencer.adopt(&block);
        inner.state = new_state;
        inner.blocks.push(block);
        inner.block_txs.push(Vec::new());
        self.storage.save(&inner.state);
    }

    fn account_view(&self, id: &Identity) -> AccountView {
        let inner = self.inner.lock().expect("node mutex");
        let a = inner
            .state
            .accounts
            .get(id)
            .copied()
            .unwrap_or_else(|| Account::new(*id));
        AccountView {
            balance: a.balance.to_string(),
            escrowed: a.escrowed.to_string(),
            bonded: a.bonded.to_string(),
        }
    }

    fn standing_view(&self, id: &Identity) -> StandingView {
        let inner = self.inner.lock().expect("node mutex");
        match inner.state.standing.get(id) {
            Some(s) => StandingView {
                sp: s.sp.to_string(),
                strikes: s.strikes,
            },
            None => StandingView {
                sp: "0".to_string(),
                strikes: 0,
            },
        }
    }

    fn head(&self) -> HeadView {
        let inner = self.inner.lock().expect("node mutex");
        HeadView {
            height: inner.sequencer.height(),
            state_root: hex::encode(inner.state.state_root()),
            epoch: inner.sequencer.epoch(),
        }
    }

    fn task_view(&self, task: &TaskId) -> Option<TaskView> {
        let inner = self.inner.lock().expect("node mutex");
        let submission = inner.state.tasks.get(task).cloned()?;
        let status = format!("{:?}", submission.status);
        Some(TaskView {
            submission,
            status,
            receipt: inner.state.receipts.get(task).cloned(),
            q_entry: inner.state.q_ledger.get(task).cloned(),
        })
    }

    fn q_entry(&self, task: &TaskId) -> Option<QLedgerEntry> {
        let inner = self.inner.lock().expect("node mutex");
        inner.state.q_ledger.get(task).cloned()
    }

    fn task_claim_stake(&self, task: &TaskId) -> Option<Ucu> {
        let inner = self.inner.lock().expect("node mutex");
        inner.state.tasks.get(task).map(|s| s.claim_stake)
    }

    fn block_at(&self, height: u64) -> Option<Block> {
        if height == 0 {
            return None;
        }
        let inner = self.inner.lock().expect("node mutex");
        inner.blocks.get((height - 1) as usize).cloned()
    }

    fn block_txs_at(&self, height: u64) -> Option<Vec<SignedTx>> {
        if height == 0 {
            return None;
        }
        let inner = self.inner.lock().expect("node mutex");
        inner.block_txs.get((height - 1) as usize).cloned()
    }

    fn put_blob(&self, bytes: Vec<u8>) -> ContentId {
        let id = content_id(&bytes);
        let mut inner = self.inner.lock().expect("node mutex");
        inner.blobs.insert(id, bytes);
        id
    }

    fn get_blob(&self, id: &ContentId) -> Option<Vec<u8>> {
        let inner = self.inner.lock().expect("node mutex");
        inner.blobs.get(id).cloned()
    }

    /// Advisory local metering for a content-addressed `(program, input)` pair.
    fn estimate(&self, program: &ContentId, input: &ContentId) -> Result<Ucu, EstimateError> {
        let (prog, inp) = {
            let inner = self.inner.lock().expect("node mutex");
            (
                inner.blobs.get(program).cloned(),
                inner.blobs.get(input).cloned(),
            )
        };
        let prog = prog.ok_or(EstimateError::MissingProgram)?;
        let inp = inp.ok_or(EstimateError::MissingInput)?;
        Ok(self.dvm.meter(&prog, &inp, &self.benchmark))
    }
}

enum EstimateError {
    MissingProgram,
    MissingInput,
}

// ============================ RPC views ====================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTaskResp {
    pub task_id: String,
    pub escrowed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimResp {
    pub ok: bool,
    pub claim_stake: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkResp {
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountView {
    pub balance: String,
    pub escrowed: String,
    pub bonded: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StandingView {
    pub sp: String,
    pub strikes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadView {
    pub height: u64,
    pub state_root: String,
    pub epoch: u64,
}

/// `getTask` response: the Submission, its current status, the Receipt (if settled),
/// and the (𝕌, ℚ) ledger entry (spec/09 §8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskView {
    pub submission: Submission,
    pub status: String,
    pub receipt: Option<Receipt>,
    pub q_entry: Option<QLedgerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimateResp {
    pub ucu: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PutBlobResp {
    pub content_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBlobResp {
    pub data: String,
}

// ============================ RPC API ======================================

/// The binding JSON-RPC API (spec/bindings/05 §3). State-changing methods
/// take a single positional `SignedTx`; read methods take positional scalar args.
#[rpc(server, client, namespace_separator = "_")]
pub trait DucpApi {
    #[method(name = "ducp_submitTask")]
    fn submit_task(&self, tx: SignedTx) -> RpcResult<SubmitTaskResp>;

    #[method(name = "ducp_claimTask")]
    fn claim_task(&self, tx: SignedTx) -> RpcResult<ClaimResp>;

    #[method(name = "ducp_submitProof")]
    fn submit_proof(&self, tx: SignedTx) -> RpcResult<OkResp>;

    #[method(name = "ducp_transfer")]
    fn transfer(&self, tx: SignedTx) -> RpcResult<OkResp>;

    #[method(name = "ducp_challenge")]
    fn challenge(&self, tx: SignedTx) -> RpcResult<OkResp>;

    #[method(name = "ducp_getTask")]
    fn get_task(&self, task_id: String) -> RpcResult<TaskView>;

    #[method(name = "ducp_getAccount")]
    fn get_account(&self, id: String) -> RpcResult<AccountView>;

    #[method(name = "ducp_getStanding")]
    fn get_standing(&self, id: String) -> RpcResult<StandingView>;

    #[method(name = "ducp_getHead")]
    fn get_head(&self) -> RpcResult<HeadView>;

    #[method(name = "ducp_getBlock")]
    fn get_block(&self, height: u64) -> RpcResult<Block>;

    #[method(name = "ducp_getBlockTxs")]
    fn get_block_txs(&self, height: u64) -> RpcResult<Vec<SignedTx>>;

    #[method(name = "ducp_getQEntry")]
    fn get_q_entry(&self, task_id: String) -> RpcResult<QLedgerEntry>;

    #[method(name = "ducp_estimateUcu")]
    fn estimate_ucu(
        &self,
        program: String,
        input: String,
        benchmark: u32,
    ) -> RpcResult<EstimateResp>;

    #[method(name = "ducp_putBlob")]
    fn put_blob(&self, data: String) -> RpcResult<PutBlobResp>;

    #[method(name = "ducp_getBlob")]
    fn get_blob(&self, content_id: String) -> RpcResult<GetBlobResp>;
}

/// The server implementation backed by a [`NodeHandle`].
pub struct RpcServerImpl {
    handle: Arc<NodeHandle>,
}

impl RpcServerImpl {
    pub fn new(handle: Arc<NodeHandle>) -> Self {
        RpcServerImpl { handle }
    }
}

impl DucpApiServer for RpcServerImpl {
    fn submit_task(&self, tx: SignedTx) -> RpcResult<SubmitTaskResp> {
        let body = match &tx.tx {
            Tx::SubmitTask(b) => b.clone(),
            _ => return Err(invalid("expected a SubmitTask transaction")),
        };
        let task_id = body.task_id();
        let escrowed = body.limits.max_ucu + self.handle.params.fee(body.limits.max_ucu);
        self.handle.submit(tx).map_err(reject_err)?;
        Ok(SubmitTaskResp {
            task_id: hex::encode(task_id),
            escrowed: escrowed.to_string(),
        })
    }

    fn claim_task(&self, tx: SignedTx) -> RpcResult<ClaimResp> {
        let task = match &tx.tx {
            Tx::ClaimTask { task } => *task,
            _ => return Err(invalid("expected a ClaimTask transaction")),
        };
        self.handle.submit(tx).map_err(reject_err)?;
        let claim_stake = self.handle.task_claim_stake(&task).unwrap_or(0);
        Ok(ClaimResp {
            ok: true,
            claim_stake: claim_stake.to_string(),
        })
    }

    fn submit_proof(&self, tx: SignedTx) -> RpcResult<OkResp> {
        if !matches!(tx.tx, Tx::SubmitProof(_)) {
            return Err(invalid("expected a SubmitProof transaction"));
        }
        self.handle.submit(tx).map_err(reject_err)?;
        Ok(OkResp { ok: true })
    }

    fn transfer(&self, tx: SignedTx) -> RpcResult<OkResp> {
        if !matches!(tx.tx, Tx::Transfer { .. }) {
            return Err(invalid("expected a Transfer transaction"));
        }
        self.handle.submit(tx).map_err(reject_err)?;
        Ok(OkResp { ok: true })
    }

    fn challenge(&self, tx: SignedTx) -> RpcResult<OkResp> {
        let task = match &tx.tx {
            Tx::Challenge { task, .. } => *task,
            _ => return Err(invalid("expected a Challenge transaction")),
        };
        // Lock the bond + record the challenge, then re-execute and resolve.
        self.handle.submit(tx).map_err(reject_err)?;
        self.handle
            .resolve_pending_challenge(task)
            .map_err(reject_err)?;
        Ok(OkResp { ok: true })
    }

    fn get_task(&self, task_id: String) -> RpcResult<TaskView> {
        let task = parse_hash(&task_id)?;
        self.handle
            .task_view(&task)
            .ok_or_else(|| not_found("task"))
    }

    fn get_account(&self, id: String) -> RpcResult<AccountView> {
        let id = parse_hash(&id)?;
        Ok(self.handle.account_view(&id))
    }

    fn get_standing(&self, id: String) -> RpcResult<StandingView> {
        let id = parse_hash(&id)?;
        Ok(self.handle.standing_view(&id))
    }

    fn get_head(&self) -> RpcResult<HeadView> {
        Ok(self.handle.head())
    }

    fn get_block(&self, height: u64) -> RpcResult<Block> {
        self.handle
            .block_at(height)
            .ok_or_else(|| not_found("block"))
    }

    fn get_block_txs(&self, height: u64) -> RpcResult<Vec<SignedTx>> {
        self.handle
            .block_txs_at(height)
            .ok_or_else(|| not_found("block"))
    }

    fn get_q_entry(&self, task_id: String) -> RpcResult<QLedgerEntry> {
        let task = parse_hash(&task_id)?;
        self.handle
            .q_entry(&task)
            .ok_or_else(|| not_found("q-entry"))
    }

    fn estimate_ucu(
        &self,
        program: String,
        input: String,
        _benchmark: u32,
    ) -> RpcResult<EstimateResp> {
        let program = parse_hash(&program)?;
        let input = parse_hash(&input)?;
        match self.handle.estimate(&program, &input) {
            Ok(ucu) => Ok(EstimateResp {
                ucu: ucu.to_string(),
            }),
            Err(EstimateError::MissingProgram) => Err(not_found("program blob")),
            Err(EstimateError::MissingInput) => Err(not_found("input blob")),
        }
    }

    fn put_blob(&self, data: String) -> RpcResult<PutBlobResp> {
        let bytes = parse_hex(&data)?;
        let id = self.handle.put_blob(bytes);
        Ok(PutBlobResp {
            content_id: hex::encode(id),
        })
    }

    fn get_blob(&self, content_id: String) -> RpcResult<GetBlobResp> {
        let id = parse_hash(&content_id)?;
        let bytes = self.handle.get_blob(&id).ok_or_else(|| not_found("blob"))?;
        Ok(GetBlobResp {
            data: hex::encode(bytes),
        })
    }
}

/// Start the JSON-RPC server, returning the bound address and a handle that keeps
/// the server alive while held.
pub async fn start_server(
    handle: Arc<NodeHandle>,
    addr: SocketAddr,
) -> anyhow::Result<(SocketAddr, ServerHandle)> {
    let server = Server::builder().build(addr).await?;
    let bound = server.local_addr()?;
    let module = RpcServerImpl::new(handle).into_rpc();
    let server_handle = server.start(module);
    Ok((bound, server_handle))
}

// ============================ Error mapping ================================

const ERR_BASE: i32 = -32000;

fn reject_code(r: &Reject) -> i32 {
    // Stable, distinct codes per Reject variant (05 §3).
    let offset = match r {
        Reject::BadSignature => 1,
        Reject::BadNonce => 2,
        Reject::UnknownAccount => 3,
        Reject::UnknownTask => 4,
        Reject::InsufficientBalance => 5,
        Reject::BadStatus => 6,
        Reject::DeadlinePassed => 7,
        Reject::AlreadyClaimed => 8,
        Reject::WrongProvider => 9,
        Reject::UcuExceedsLimit => 10,
        Reject::BenchmarkMismatch => 11,
        Reject::NotInClawbackWindow => 12,
        Reject::BondTooSmall => 13,
        Reject::UnsupportedFeature => 14,
        Reject::ConservationViolated => 15,
        Reject::Invalid => 16,
    };
    ERR_BASE - offset
}

fn reject_err(r: Reject) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(reject_code(&r), r.to_string(), None::<()>)
}

fn invalid(msg: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(-32602, msg.to_string(), None::<()>)
}

fn not_found(what: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(-32004, format!("{what} not found"), None::<()>)
}

fn parse_hex(s: &str) -> RpcResult<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).map_err(|e| invalid(&format!("bad hex: {e}")))
}

fn parse_hash(s: &str) -> RpcResult<Hash> {
    let v = parse_hex(s)?;
    v.try_into().map_err(|_| invalid("expected 32-byte hex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }

    #[test]
    fn parse_hash_roundtrip() {
        let h = [7u8; 32];
        assert_eq!(parse_hash(&hex::encode(h)).unwrap(), h);
        assert!(parse_hash("zz").is_err());
    }

    #[test]
    fn reject_codes_are_distinct() {
        let variants = [
            Reject::BadSignature,
            Reject::BadNonce,
            Reject::InsufficientBalance,
            Reject::BadStatus,
            Reject::Invalid,
        ];
        let codes: Vec<i32> = variants.iter().map(reject_code).collect();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(codes.len(), sorted.len());
    }
}
