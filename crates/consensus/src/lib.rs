//! # ducp-consensus
//!
//! Transaction ordering and finality. The reference-node binding uses [`SingleSequencer`]: one
//! designated node orders admitted txs (FIFO by arrival, ties by `TxId`), applies
//! the ledger transition in order, and commits a `state_root`. Other nodes
//! **replay** [`SingleSequencer::commit`] and MUST reach the identical root — this
//! is state-machine replication, so the devnet is verifiable even with one proposer
//! (spec/bindings/04 §6). A BFT engine is a later `impl ConsensusEngine` with
//! no change to the ledger.
//!
//! Specification: <https://github.com/ducp-protocol/ducp-spec>
//! Status: Reference implementation for DUCP-SPEC v0.2.0.

use ducp_governance::Params;
use ducp_ledger::{apply, State};
use ducp_types::{Block, Epoch, Hash, Identity, Reject, SignedTx, TxId};

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Per-transaction outcome from a [`Proposal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxResult {
    pub tx_id: TxId,
    pub accepted: bool,
    pub reject: Option<Reject>,
}

/// The result of producing a block: the block, the accepted transactions (in block
/// order), the post-state, and the per-transaction outcomes.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub block: Block,
    pub txs: Vec<SignedTx>,
    pub state: State,
    pub results: Vec<TxResult>,
}

/// The consensus interface (spec/bindings/04 §6). `produce` orders + applies
/// admitted txs into a candidate block; `commit` deterministically replays a block,
/// which is how replicas reach the identical `state_root`.
pub trait ConsensusEngine {
    /// Order and apply admitted txs into a candidate block on top of `state`.
    fn produce(&self, mempool: &[SignedTx], state: &State, params: &Params) -> Proposal;

    /// Replay a block's transactions on `state`, returning the new state. Fails if
    /// the resolved txs do not match `block.txs` or the recomputed `state_root`
    /// differs from `block.state_root`.
    fn commit(
        &self,
        block: &Block,
        txs: &[SignedTx],
        state: &State,
        params: &Params,
    ) -> Result<State, Reject>;
}

/// The binding single-sequencer engine. Tracks the chain head (`height`,
/// `parent`) and the current `epoch`. `produce`/`commit` are pure with respect to
/// the head; the head advances only via [`SingleSequencer::adopt`].
#[derive(Debug, Clone)]
pub struct SingleSequencer {
    proposer: Identity,
    height: u64,
    parent: Hash,
    epoch: Epoch,
}

impl SingleSequencer {
    /// New sequencer at genesis (height 0, zero parent, epoch 0).
    pub fn new(proposer: Identity) -> Self {
        SingleSequencer {
            proposer,
            height: 0,
            parent: [0u8; 32],
            epoch: 0,
        }
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn parent(&self) -> Hash {
        self.parent
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn proposer(&self) -> Identity {
        self.proposer
    }

    /// Advance the head to a just-produced/committed block.
    pub fn adopt(&mut self, block: &Block) {
        self.height = block.height;
        self.parent = block.block_hash();
        self.epoch = block.epoch;
    }

    /// Seal a system state transition (e.g. fraud resolution or epoch advance) that
    /// did not arise from user transactions, as an empty-tx block committing to the
    /// post-state's `state_root`. The caller adopts the returned block.
    pub fn seal_block(&self, post_state: &State) -> Block {
        Block {
            height: self.height + 1,
            parent: self.parent,
            epoch: self.epoch,
            txs: Vec::new(),
            state_root: post_state.state_root(),
            proposer: self.proposer,
        }
    }

    /// Advance the epoch boundary: apply Standing decay and bond release via the
    /// ledger, returning the new state. The next produced block carries the new
    /// epoch.
    pub fn advance_epoch(&mut self, state: &State, params: &Params) -> State {
        let s = ducp_ledger::advance_epoch(state, params);
        self.epoch = s.epoch;
        s
    }

    fn next_epoch_for_block(&self) -> Epoch {
        self.epoch
    }
}

impl ConsensusEngine for SingleSequencer {
    fn produce(&self, mempool: &[SignedTx], state: &State, params: &Params) -> Proposal {
        let mut s = state.clone();
        let mut accepted_ids: Vec<TxId> = Vec::new();
        let mut accepted_txs: Vec<SignedTx> = Vec::new();
        let mut results: Vec<TxResult> = Vec::new();

        // FIFO by arrival (mempool order); arrival is total, so no ties to break.
        for tx in mempool {
            let id = tx.tx_id();
            match apply(&s, tx, params) {
                Ok(ns) => {
                    s = ns;
                    accepted_ids.push(id);
                    accepted_txs.push(tx.clone());
                    results.push(TxResult {
                        tx_id: id,
                        accepted: true,
                        reject: None,
                    });
                }
                Err(e) => results.push(TxResult {
                    tx_id: id,
                    accepted: false,
                    reject: Some(e),
                }),
            }
        }

        let block = Block {
            height: self.height + 1,
            parent: self.parent,
            epoch: self.next_epoch_for_block(),
            txs: accepted_ids,
            state_root: s.state_root(),
            proposer: self.proposer,
        };

        Proposal {
            block,
            txs: accepted_txs,
            state: s,
            results,
        }
    }

    fn commit(
        &self,
        block: &Block,
        txs: &[SignedTx],
        state: &State,
        params: &Params,
    ) -> Result<State, Reject> {
        if txs.len() != block.txs.len() {
            return Err(Reject::Invalid);
        }
        let mut s = state.clone();
        for (tx, expected_id) in txs.iter().zip(block.txs.iter()) {
            if tx.tx_id() != *expected_id {
                return Err(Reject::Invalid);
            }
            // Every tx in a committed block was accepted by the proposer, so a
            // rejection here means divergence.
            s = apply(&s, tx, params)?;
        }
        if s.state_root() != block.state_root {
            return Err(Reject::Invalid);
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ducp_types::{keys, Tx, UCU_SCALE};

    fn seed(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }

    #[test]
    fn produce_then_replica_commit_reach_identical_root() {
        let params = Params::devnet();
        let alice = keys::identity(&seed(1));
        let bob = keys::identity(&seed(2));
        let state = State::genesis(&[(alice, 100 * UCU_SCALE)], 0);

        let seq = SingleSequencer::new(alice);
        let tx = SignedTx::sign(
            &seed(1),
            Tx::Transfer {
                to: bob,
                amount: 10 * UCU_SCALE,
            },
            0,
        );
        let proposal = seq.produce(std::slice::from_ref(&tx), &state, &params);
        assert_eq!(proposal.block.txs.len(), 1);
        assert!(proposal.results[0].accepted);

        // A replica replays the same block and reaches the identical root.
        let replica = SingleSequencer::new(alice);
        let replayed = replica
            .commit(&proposal.block, &proposal.txs, &state, &params)
            .unwrap();
        assert_eq!(replayed.state_root(), proposal.block.state_root);
        assert_eq!(replayed.state_root(), proposal.state.state_root());
        assert_eq!(replayed.balance(&bob), 10 * UCU_SCALE);
    }

    #[test]
    fn rejected_tx_is_excluded_from_block() {
        let params = Params::devnet();
        let alice = keys::identity(&seed(1));
        let bob = keys::identity(&seed(2));
        let state = State::genesis(&[(alice, 5 * UCU_SCALE)], 0);
        let seq = SingleSequencer::new(alice);

        // Alice tries to send more than she has.
        let tx = SignedTx::sign(
            &seed(1),
            Tx::Transfer {
                to: bob,
                amount: 10 * UCU_SCALE,
            },
            0,
        );
        let proposal = seq.produce(&[tx], &state, &params);
        assert!(proposal.block.txs.is_empty());
        assert!(!proposal.results[0].accepted);
        assert_eq!(
            proposal.results[0].reject,
            Some(Reject::InsufficientBalance)
        );
    }

    #[test]
    fn adopt_advances_the_head() {
        let params = Params::devnet();
        let alice = keys::identity(&seed(1));
        let state = State::genesis(&[(alice, 100 * UCU_SCALE)], 0);
        let mut seq = SingleSequencer::new(alice);
        assert_eq!(seq.height(), 0);

        let tx = SignedTx::sign(
            &seed(1),
            Tx::Transfer {
                to: [9u8; 32],
                amount: 1,
            },
            0,
        );
        let p = seq.produce(&[tx], &state, &params);
        seq.adopt(&p.block);
        assert_eq!(seq.height(), 1);
        assert_eq!(seq.parent(), p.block.block_hash());
    }
}
