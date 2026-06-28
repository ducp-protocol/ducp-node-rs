//! # ducp-ledger
//!
//! The deterministic ledger state machine: accounts and 𝕌, the separate Standing
//! reputation ledger, and the on-chain **ℚ-ledger** (spec/implementation/04,
//! spec/09 §7). [`apply`] is a pure transition `State × SignedTx → State`.
//!
//! Base reward is strictly **𝕌-proportional**. The efficiency multiplier (DP-0001,
//! spec/09) is the only place ℚ could touch accrual — and in Profile 0 it is fixed
//! at 1.0, so ℚ is recorded but inert (`I-Q-REWARDNEUTRAL`). Every settled task
//! records a `(𝕌, ℚ)` entry from genesis, with ℚ null in Profile 0 (`I-Q-NULL`).
//!
//! Conservation (`I-LEDGER-CONSERVE`): after every transition,
//! `Σ(balance + escrowed + bonded) + fee_pool == minted − burned`.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//! Status: Profile 0 implementation for spec v0.2.0.

use borsh::{BorshDeserialize, BorshSerialize};
use ducp_governance::Params;
use ducp_types::{
    hash_canonical, Account, BenchmarkVersion, ComputeProof, Epoch, Hash, Identity, IrId,
    QLedgerEntry, Receipt, Reject, SignedTx, Sp, StandingRecord, Submission, TaskBody, TaskId,
    TaskStatus, Tx, Ucu, VerificationTier,
};
use std::collections::{BTreeMap, BTreeSet};

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Total minted / burned 𝕌 (audit). Circulating = `minted − burned`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize)]
pub struct Supply {
    pub minted: Ucu,
    pub burned: Ucu,
}

/// An open challenge against a settled task: the challenger and their posted bond
/// (spec/implementation/03 §3). Resolved by re-execution within the clawback window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ChallengeRecord {
    pub challenger: Identity,
    pub bond: Ucu,
}

/// The full ledger state. All maps are sorted (`BTreeMap`) so the canonical
/// encoding — and thus [`State::state_root`] — is deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize)]
pub struct State {
    pub accounts: BTreeMap<Identity, Account>,
    pub standing: BTreeMap<Identity, StandingRecord>,
    /// Task bodies, kept so claim/proof can read limits, deadline, and benchmark.
    pub bodies: BTreeMap<TaskId, TaskBody>,
    pub tasks: BTreeMap<TaskId, Submission>,
    pub proofs: BTreeMap<TaskId, ComputeProof>,
    pub receipts: BTreeMap<TaskId, Receipt>,
    /// The on-chain ℚ-ledger: `(𝕌, ℚ)` per settled task (spec/09 §7).
    pub q_ledger: BTreeMap<TaskId, QLedgerEntry>,
    /// Open challenges awaiting resolution (keyed by task).
    pub pending_challenges: BTreeMap<TaskId, ChallengeRecord>,
    /// Tasks already slashed for fraud (idempotence / no double-slash).
    pub slashed: BTreeSet<TaskId>,
    /// Tasks whose claim stake has been released after the clawback window.
    pub released: BTreeSet<TaskId>,
    /// Per-account transaction nonce (replay protection).
    pub nonces: BTreeMap<Identity, u64>,
    pub supply: Supply,
    /// Accumulated validator fees (claimable by the sequencer/validators).
    pub fee_pool: Ucu,
    pub epoch: Epoch,
}

impl State {
    /// Build a genesis state that mints initial balances to the given accounts.
    pub fn genesis(allocations: &[(Identity, Ucu)], epoch: Epoch) -> State {
        let mut s = State {
            epoch,
            ..Default::default()
        };
        let mut minted: Ucu = 0;
        for (id, amount) in allocations {
            let a = s.acct_mut(*id);
            a.balance += *amount;
            minted += *amount;
        }
        s.supply.minted = minted;
        debug_assert!(s.check_conservation());
        s
    }

    /// The 𝕌 commitment to the whole state: `BLAKE3(canonical(State))` (provisional;
    /// a Merkle commitment replaces it later — spec/implementation/04 §6).
    pub fn state_root(&self) -> Hash {
        hash_canonical(self)
    }

    /// Spendable balance of an account (0 if unknown).
    pub fn balance(&self, id: &Identity) -> Ucu {
        self.accounts.get(id).map(|a| a.balance).unwrap_or(0)
    }

    /// Current Standing of an identity (0 if unknown).
    pub fn standing_of(&self, id: &Identity) -> Sp {
        self.standing.get(id).map(|s| s.sp).unwrap_or(0)
    }

    /// `I-LEDGER-CONSERVE`: total held 𝕌 equals circulating supply.
    pub fn check_conservation(&self) -> bool {
        let mut held: u128 = self.fee_pool;
        for a in self.accounts.values() {
            held += a.balance + a.escrowed + a.bonded;
        }
        held == self.supply.minted.saturating_sub(self.supply.burned)
    }

    // ---- internal mutators (operate on the working copy inside `apply`) ----

    fn acct_mut(&mut self, id: Identity) -> &mut Account {
        self.accounts.entry(id).or_insert_with(|| Account::new(id))
    }

    fn standing_mut(&mut self, id: Identity) -> &mut StandingRecord {
        let epoch = self.epoch;
        self.standing
            .entry(id)
            .or_insert_with(|| StandingRecord::new(id, epoch))
    }

    fn check_and_bump_nonce(&mut self, author: Identity, nonce: u64) -> Result<(), Reject> {
        let expected = self.nonces.get(&author).copied().unwrap_or(0);
        if nonce != expected {
            return Err(Reject::BadNonce);
        }
        self.nonces.insert(author, expected + 1);
        Ok(())
    }

    fn submit_task(
        &mut self,
        requester: Identity,
        body: &TaskBody,
        params: &Params,
    ) -> Result<(), Reject> {
        if body.ir != IrId::Wasm || body.tier != VerificationTier::SampledReexec {
            return Err(Reject::Invalid);
        }
        let task = body.task_id();
        if self.tasks.contains_key(&task) {
            return Err(Reject::Invalid); // duplicate submission
        }
        let max_ucu = body.limits.max_ucu;
        let fee = params.fee(max_ucu);
        let need = max_ucu.checked_add(fee).ok_or(Reject::Invalid)?;
        {
            let acct = self.acct_mut(requester);
            if acct.balance < need {
                return Err(Reject::InsufficientBalance);
            }
            acct.balance -= need;
            acct.escrowed += need;
        }
        self.bodies.insert(task, body.clone());
        self.tasks.insert(
            task,
            Submission {
                task,
                requester,
                ucu_count: 0,
                fee,
                status: TaskStatus::Submitted,
                provider: None,
                claim_stake: 0,
            },
        );
        Ok(())
    }

    fn claim_task(
        &mut self,
        provider: Identity,
        task: TaskId,
        params: &Params,
    ) -> Result<(), Reject> {
        let body = self.bodies.get(&task).ok_or(Reject::UnknownTask)?.clone();
        let status = self.tasks.get(&task).ok_or(Reject::UnknownTask)?.status;
        if status != TaskStatus::Submitted {
            return Err(Reject::BadStatus);
        }
        if self.epoch > body.deadline {
            return Err(Reject::DeadlinePassed);
        }
        let stake = params.claim_stake(body.limits.max_ucu, self.standing_of(&provider));
        {
            let acct = self.acct_mut(provider);
            if acct.balance < stake {
                return Err(Reject::InsufficientBalance);
            }
            acct.balance -= stake;
            acct.bonded += stake;
        }
        let sub = self.tasks.get_mut(&task).expect("checked above");
        sub.provider = Some(provider);
        sub.status = TaskStatus::Executing;
        sub.claim_stake = stake;
        Ok(())
    }

    fn submit_proof(
        &mut self,
        author: Identity,
        proof: &ComputeProof,
        params: &Params,
    ) -> Result<(), Reject> {
        let body = self
            .bodies
            .get(&proof.task)
            .ok_or(Reject::UnknownTask)?
            .clone();
        let sub = self
            .tasks
            .get(&proof.task)
            .ok_or(Reject::UnknownTask)?
            .clone();
        if sub.status != TaskStatus::Executing {
            return Err(Reject::BadStatus);
        }
        if sub.provider != Some(author) || proof.provider != author {
            return Err(Reject::WrongProvider);
        }
        if proof.ucu_count > body.limits.max_ucu {
            return Err(Reject::UcuExceedsLimit);
        }
        if proof.benchmark != body.benchmark {
            return Err(Reject::BenchmarkMismatch);
        }

        // Record the proof and mark verified (no re-execution here — 03 §1).
        self.proofs.insert(proof.task, proof.clone());
        {
            let s = self.tasks.get_mut(&proof.task).expect("present");
            s.status = TaskStatus::Verified;
            s.ucu_count = proof.ucu_count;
        }

        // Settle atomically (04 §3).
        self.settle(&body, &sub, proof, params);
        Ok(())
    }

    /// Atomic settlement on `Verified` (04 §3). All effects apply together.
    fn settle(&mut self, body: &TaskBody, sub: &Submission, proof: &ComputeProof, params: &Params) {
        let requester = sub.requester;
        let provider = proof.provider;
        let fee = sub.fee;
        let max_ucu = body.limits.max_ucu;
        let u = proof.ucu_count;

        // 1–3: drain the requester's escrow (payment + refund + fee).
        {
            let r = self.acct_mut(requester);
            r.escrowed -= max_ucu + fee;
            r.balance += max_ucu - u; // refund the unused ceiling
        }
        self.fee_pool += fee; // 3: validator fee

        // 1: payment transfer (not burned/reminted — I-ECON-TRANSFER).
        {
            let p = self.acct_mut(provider);
            p.balance += u;
        }

        // 4: work-issuance (the only mint — I-ECON-ONEMINT).
        let w = params.issuance(u);
        {
            let p = self.acct_mut(provider);
            p.balance += w;
        }
        self.supply.minted += w;

        // 5: Standing accrual (efficiency_mult = 1.0 in P0).
        let delta = params.standing_accrual(u, params.efficiency_mult_ppm);
        {
            let st = self.standing_mut(provider);
            st.sp += delta;
        }

        // 6: bond stays locked (already in provider.bonded) until clawback_until.
        let clawback_until = self.epoch + params.clawback_epochs;

        // 7: write the immutable Receipt and finalize status.
        self.receipts.insert(
            proof.task,
            Receipt {
                task: proof.task,
                paid_to_provider: u,
                work_issuance: w,
                validator_fee: fee,
                standing_delta: delta,
                settled_epoch: self.epoch,
                clawback_until,
            },
        );
        {
            let s = self.tasks.get_mut(&proof.task).expect("present");
            s.status = TaskStatus::Settled;
        }

        // ℚ-ledger genesis MUST (spec/09 §7.1): record (𝕌, ℚ). In Profile 0 the
        // EnergyAttestor is Null, so ℚ is null regardless of any `power_seal`
        // (I-Q-NULL, I-Q-REWARDNEUTRAL).
        self.q_ledger.insert(
            proof.task,
            QLedgerEntry::unmeasured(proof.task, u, body.benchmark),
        );
    }

    /// Open a challenge against a settled task within its clawback window
    /// (spec/implementation/03 §3): lock the challenger's bond and record it.
    fn open_challenge(
        &mut self,
        challenger: Identity,
        task: TaskId,
        bond: Ucu,
        params: &Params,
    ) -> Result<(), Reject> {
        let receipt = self.receipts.get(&task).ok_or(Reject::UnknownTask)?.clone();
        let status = self.tasks.get(&task).ok_or(Reject::UnknownTask)?.status;
        if status != TaskStatus::Settled {
            return Err(Reject::BadStatus);
        }
        if self.epoch > receipt.clawback_until {
            return Err(Reject::NotInClawbackWindow);
        }
        if self.slashed.contains(&task) || self.pending_challenges.contains_key(&task) {
            return Err(Reject::Invalid);
        }
        if bond < params.bond_min(receipt.paid_to_provider) {
            return Err(Reject::BondTooSmall);
        }
        {
            let ca = self.acct_mut(challenger);
            if ca.balance < bond {
                return Err(Reject::InsufficientBalance);
            }
            ca.balance -= bond;
            ca.bonded += bond;
        }
        self.pending_challenges
            .insert(task, ChallengeRecord { challenger, bond });
        Ok(())
    }

    fn transfer(&mut self, from: Identity, to: Identity, amount: Ucu) -> Result<(), Reject> {
        {
            let f = self.acct_mut(from);
            if f.balance < amount {
                return Err(Reject::InsufficientBalance);
            }
            f.balance -= amount;
        }
        self.acct_mut(to).balance += amount;
        Ok(())
    }
}

/// Apply a signed transaction deterministically, returning the new state. Pure: on
/// `Err`, the input state is unchanged (the working copy is discarded).
/// Signature and nonce are checked first (04 §2).
pub fn apply(state: &State, stx: &SignedTx, params: &Params) -> Result<State, Reject> {
    if !stx.verify_sig() {
        return Err(Reject::BadSignature);
    }
    let mut s = state.clone();
    s.check_and_bump_nonce(stx.author, stx.nonce)?;
    match &stx.tx {
        Tx::SubmitTask(body) => s.submit_task(stx.author, body, params)?,
        Tx::ClaimTask { task } => s.claim_task(stx.author, *task, params)?,
        Tx::SubmitProof(proof) => s.submit_proof(stx.author, proof, params)?,
        Tx::Transfer { to, amount } => s.transfer(stx.author, *to, *amount)?,
        Tx::Challenge { task, bond } => s.open_challenge(stx.author, *task, *bond, params)?,
    }
    debug_assert!(
        s.check_conservation(),
        "I-LEDGER-CONSERVE violated by transition"
    );
    Ok(s)
}

/// Advance the epoch boundary: apply Standing decay deterministically to every
/// identity (`I-STAND-DECAY`), then release any claim stake whose clawback window
/// has closed without a successful challenge (spec/implementation/04 §3). The
/// settled Receipt is never rewritten — release is a stake movement, not a reversal
/// (`I-ECON-FINAL`).
pub fn advance_epoch(state: &State, params: &Params) -> State {
    let mut s = state.clone();
    s.epoch += 1;

    for st in s.standing.values_mut() {
        st.sp = params.decay(st.sp);
        st.last_decay_epoch = s.epoch;
    }

    // Release matured bonds. Collect first to avoid borrowing `s` while mutating.
    let mut to_release: Vec<(TaskId, Identity, Ucu)> = Vec::new();
    for (task, receipt) in &s.receipts {
        if receipt.clawback_until <= s.epoch
            && !s.released.contains(task)
            && !s.slashed.contains(task)
        {
            if let Some(sub) = s.tasks.get(task) {
                if let Some(provider) = sub.provider {
                    to_release.push((*task, provider, sub.claim_stake));
                }
            }
        }
    }
    for (task, provider, stake) in to_release {
        let amt = {
            let pa = s.acct_mut(provider);
            let a = stake.min(pa.bonded);
            pa.bonded -= a;
            pa.balance += a;
            a
        };
        let _ = amt;
        s.released.insert(task);
    }

    debug_assert!(s.check_conservation());
    s
}

/// Advance the epoch repeatedly until `target` (convenience for tests and the node).
pub fn advance_to_epoch(state: &State, target: Epoch, params: &Params) -> State {
    let mut s = state.clone();
    while s.epoch < target {
        s = advance_epoch(&s, params);
    }
    s
}

/// Convenience: build the Profile 0 reward-neutral ℚ-ledger entry for a settled task
/// (𝕌 recorded, ℚ null — `I-Q-NULL`).
pub fn q_ledger_entry_p0(task: TaskId, ucu: Ucu, benchmark: BenchmarkVersion) -> QLedgerEntry {
    QLedgerEntry::unmeasured(task, ucu, benchmark)
}

/// Apply the penalties for proven fraud on a settled task (spec/implementation/04
/// §4): clawback the payment from bonded stake, burn the work-issuance, slash a
/// fine (rewarding the auditor), and floor the offender's Standing. Economic
/// reversal is via **stake**, never by rewriting the settled tx (`I-ECON-FINAL`).
/// `reward_to` is the auditor/Challenger; `None` routes the reward to the fee pool
/// (sampling audits). Idempotent: a task is slashed at most once. All amounts are
/// clamped to what is available so conservation holds exactly.
pub fn resolve_fraud(
    state: &State,
    task: TaskId,
    reward_to: Option<Identity>,
    params: &Params,
) -> State {
    let mut s = state.clone();
    if s.slashed.contains(&task) {
        return s;
    }
    let receipt = match s.receipts.get(&task) {
        Some(r) => r.clone(),
        None => return s,
    };
    let provider = match s.tasks.get(&task).and_then(|t| t.provider) {
        Some(p) => p,
        None => return s,
    };
    let requester = s.tasks[&task].requester;
    let p = receipt.paid_to_provider;
    let w = receipt.work_issuance;

    // 1. Clawback P from the Provider's bonded stake → Requester.
    let recovered = {
        let pa = s.acct_mut(provider);
        let r = p.min(pa.bonded);
        pa.bonded -= r;
        r
    };
    s.acct_mut(requester).balance += recovered;

    // 2. Offsetting burn of the work-issuance W (`I-ECON-BACKED`).
    let burn_w = {
        let pa = s.acct_mut(provider);
        let b = w.min(pa.balance);
        pa.balance -= b;
        b
    };
    s.supply.burned += burn_w;

    // 3. Fine F from bonded; reward R ≤ F to the auditor; remainder burned.
    let f = params.fine(p);
    let fine_avail = {
        let pa = s.acct_mut(provider);
        let fa = f.min(pa.bonded);
        pa.bonded -= fa;
        fa
    };
    let reward = params.challenger_reward(f).min(fine_avail);
    match reward_to {
        Some(id) => s.acct_mut(id).balance += reward,
        None => s.fee_pool += reward,
    }
    s.supply.burned += fine_avail - reward;

    // 4. Standing floored + strike (escalating).
    {
        let st = s.standing_mut(provider);
        st.sp = 0;
        st.strikes += 1;
    }

    s.slashed.insert(task);
    if let Some(t) = s.tasks.get_mut(&task) {
        t.status = TaskStatus::Failed;
    }
    debug_assert!(s.check_conservation(), "fraud resolution must conserve 𝕌");
    s
}

/// Resolve an open challenge given the re-execution verdict (`fraud`). On fraud:
/// apply penalties (rewarding the challenger) and return the challenger's bond. On a
/// failed challenge: the challenger forfeits the bond (burned, anti-spam,
/// spec/implementation/03 §3).
pub fn resolve_challenge(state: &State, task: TaskId, fraud: bool, params: &Params) -> State {
    let mut s = state.clone();
    let pc = match s.pending_challenges.remove(&task) {
        Some(p) => p,
        None => return s,
    };
    if fraud {
        s = resolve_fraud(&s, task, Some(pc.challenger), params);
        // Return the challenger's bond.
        let ca = s.acct_mut(pc.challenger);
        let b = pc.bond.min(ca.bonded);
        ca.bonded -= b;
        ca.balance += b;
    } else {
        // Forfeit the bond.
        let ca = s.acct_mut(pc.challenger);
        let b = pc.bond.min(ca.bonded);
        ca.bonded -= b;
        s.supply.burned += b;
    }
    debug_assert!(s.check_conservation());
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ducp_types::{
        content_id, hash_bytes, keys, Boundary, Limits, PowerSeal, SealGrade, UCU_SCALE,
    };

    fn seed(n: u8) -> [u8; 32] {
        [n; 32]
    }

    fn make_task_body(nonce: u64) -> TaskBody {
        TaskBody {
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
            failure_policy: ducp_types::FailurePolicy::ReturnOnFailure,
            nonce,
        }
    }

    /// Drive submit → claim → proof and return the settled state.
    fn happy_path(power_seal: Option<PowerSeal>) -> (State, TaskId, Identity, Identity) {
        let params = Params::devnet();
        let req = keys::identity(&seed(1));
        let prov = keys::identity(&seed(2));
        let s = State::genesis(&[(req, 100 * UCU_SCALE), (prov, 100 * UCU_SCALE)], 0);

        let body = make_task_body(1);
        let task = body.task_id();
        let s = apply(
            &s,
            &SignedTx::sign(&seed(1), Tx::SubmitTask(body.clone()), 0),
            &params,
        )
        .unwrap();
        let s = apply(
            &s,
            &SignedTx::sign(&seed(2), Tx::ClaimTask { task }, 0),
            &params,
        )
        .unwrap();

        let result = hash_bytes(b"the-output");
        let proof = ComputeProof {
            task,
            provider: prov,
            output: content_id(b"the-output"),
            result_hash: result,
            ucu_count: 4 * UCU_SCALE,
            benchmark: 0,
            tier_data: ducp_types::TierData::SampledReexec,
            power_seal,
        };
        let s = apply(
            &s,
            &SignedTx::sign(&seed(2), Tx::SubmitProof(proof), 1),
            &params,
        )
        .unwrap();
        (s, task, req, prov)
    }

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }

    #[test]
    fn genesis_conserves() {
        let s = State::genesis(&[([1; 32], 50), ([2; 32], 70)], 0);
        assert!(s.check_conservation());
        assert_eq!(s.supply.minted, 120);
    }

    #[test]
    fn happy_path_settles_with_transfer_mint_and_standing() {
        let params = Params::devnet();
        let (s, task, req, prov) = happy_path(None);

        let sub = &s.tasks[&task];
        assert_eq!(sub.status, TaskStatus::Settled);

        let u = 4 * UCU_SCALE;
        let max_ucu = 10 * UCU_SCALE;
        let fee = params.fee(max_ucu);
        let w = params.issuance(u);
        let stake = params.claim_stake(max_ucu, 0);

        // Provider: paid u + minted w, minus the still-bonded stake.
        let prov_acct = &s.accounts[&prov];
        assert_eq!(prov_acct.balance, 100 * UCU_SCALE - stake + u + w);
        assert_eq!(prov_acct.bonded, stake);

        // Requester: escrow fully drained; refunded the unused ceiling.
        let req_acct = &s.accounts[&req];
        assert_eq!(req_acct.escrowed, 0);
        assert_eq!(
            req_acct.balance,
            100 * UCU_SCALE - (max_ucu + fee) + (max_ucu - u)
        );

        // Mint + fee accounted.
        assert_eq!(s.supply.minted, 200 * UCU_SCALE + w);
        assert_eq!(s.fee_pool, fee);

        // Standing accrued 1:1 with 𝕌.
        assert_eq!(s.standing_of(&prov), u as Sp);

        // Receipt recorded.
        let r = &s.receipts[&task];
        assert_eq!(r.paid_to_provider, u);
        assert_eq!(r.work_issuance, w);
        assert_eq!(r.clawback_until, params.clawback_epochs);

        assert!(s.check_conservation());
    }

    #[test]
    fn q_ledger_records_null_pair_at_genesis() {
        let (s, task, _, _) = happy_path(None);
        let e = &s.q_ledger[&task];
        assert_eq!(e.ucu, 4 * UCU_SCALE);
        assert_eq!(e.q, None); // I-Q-NULL
        assert_eq!(e.seal_grade, None);
    }

    #[test]
    fn reward_neutral_a_power_seal_does_not_change_settlement() {
        // I-Q-REWARDNEUTRAL: settlement is identical whether or not a power seal
        // rides the proof (the P0 ledger ignores it; ℚ stays null either way).
        let (no_seal, task_a, _, _) = happy_path(None);
        let seal = PowerSeal {
            seal_grade: SealGrade::S2Locked,
            boundary: Boundary::Chip,
            power_cap_milliwatts: 100_000,
            window_millis: 500,
            t_max_millikelvin: 300_000,
            attestation_evidence: content_id(b"evidence"),
            benchmark: 0,
        };
        let (with_seal, task_b, _, _) = happy_path(Some(seal));

        // Same task body → same task id; the economic effects (balances, supply,
        // Standing) and the ℚ entry are all identical. (The stored proof differs by
        // design — it carries the seal — so state_root differs; that is not a
        // settlement effect.)
        assert_eq!(task_a, task_b);
        assert_eq!(no_seal.accounts, with_seal.accounts);
        assert_eq!(no_seal.supply, with_seal.supply);
        assert_eq!(no_seal.fee_pool, with_seal.fee_pool);
        assert_eq!(no_seal.standing, with_seal.standing);
        assert_eq!(no_seal.q_ledger, with_seal.q_ledger);
    }

    #[test]
    fn bad_nonce_is_rejected_without_mutation() {
        let params = Params::devnet();
        let req = keys::identity(&seed(1));
        let s = State::genesis(&[(req, 100 * UCU_SCALE)], 0);
        let body = make_task_body(1);
        // Wrong nonce (expected 0).
        let bad = SignedTx::sign(&seed(1), Tx::SubmitTask(body), 5);
        assert_eq!(apply(&s, &bad, &params), Err(Reject::BadNonce));
    }

    #[test]
    fn double_claim_is_rejected() {
        let params = Params::devnet();
        let req = keys::identity(&seed(1));
        let prov1 = keys::identity(&seed(2));
        let prov2 = keys::identity(&seed(3));
        let s = State::genesis(
            &[
                (req, 100 * UCU_SCALE),
                (prov1, 100 * UCU_SCALE),
                (prov2, 100 * UCU_SCALE),
            ],
            0,
        );
        let body = make_task_body(1);
        let task = body.task_id();
        let s = apply(
            &s,
            &SignedTx::sign(&seed(1), Tx::SubmitTask(body), 0),
            &params,
        )
        .unwrap();
        let s = apply(
            &s,
            &SignedTx::sign(&seed(2), Tx::ClaimTask { task }, 0),
            &params,
        )
        .unwrap();
        let second = apply(
            &s,
            &SignedTx::sign(&seed(3), Tx::ClaimTask { task }, 0),
            &params,
        );
        assert_eq!(second, Err(Reject::BadStatus));
    }

    #[test]
    fn decay_applies_at_epoch_boundary() {
        let params = Params::devnet();
        let (s, _, _, prov) = happy_path(None);
        let before = s.standing_of(&prov);
        let s2 = advance_epoch(&s, &params);
        assert_eq!(s2.standing_of(&prov), params.decay(before));
        assert_eq!(s2.epoch, 1);
    }

    /// Settle a task and open a challenge against it from a funded challenger.
    fn settled_with_open_challenge() -> (State, TaskId, Identity, Identity, Identity) {
        let params = Params::devnet();
        let req = keys::identity(&seed(1));
        let prov = keys::identity(&seed(2));
        let chal = keys::identity(&seed(3));
        let s = State::genesis(
            &[
                (req, 100 * UCU_SCALE),
                (prov, 100 * UCU_SCALE),
                (chal, 100 * UCU_SCALE),
            ],
            0,
        );
        let body = make_task_body(1);
        let task = body.task_id();
        let s = apply(
            &s,
            &SignedTx::sign(&seed(1), Tx::SubmitTask(body), 0),
            &params,
        )
        .unwrap();
        let s = apply(
            &s,
            &SignedTx::sign(&seed(2), Tx::ClaimTask { task }, 0),
            &params,
        )
        .unwrap();
        let proof = ComputeProof {
            task,
            provider: prov,
            output: content_id(b"out"),
            result_hash: hash_bytes(b"out"),
            ucu_count: 4 * UCU_SCALE,
            benchmark: 0,
            tier_data: ducp_types::TierData::SampledReexec,
            power_seal: None,
        };
        let s = apply(
            &s,
            &SignedTx::sign(&seed(2), Tx::SubmitProof(proof), 1),
            &params,
        )
        .unwrap();
        let bond = params.bond_min(4 * UCU_SCALE);
        let s = apply(
            &s,
            &SignedTx::sign(&seed(3), Tx::Challenge { task, bond }, 0),
            &params,
        )
        .unwrap();
        (s, task, req, prov, chal)
    }

    #[test]
    fn resolve_fraud_claws_back_burns_and_floors_standing() {
        let params = Params::devnet();
        let (s, task, req, prov) = happy_path(None);
        let req_balance_before = s.accounts[&req].balance;
        let burned_before = s.supply.burned;

        let s = resolve_fraud(&s, task, None, &params);

        // Payment clawed back to the requester (from bonded stake).
        assert!(s.accounts[&req].balance > req_balance_before);
        // Work-issuance burned (supply backed — I-ECON-BACKED).
        assert!(s.supply.burned > burned_before);
        // Standing floored + strike.
        assert_eq!(s.standing_of(&prov), 0);
        assert_eq!(s.standing[&prov].strikes, 1);
        // Task marked failed; idempotent.
        assert_eq!(s.tasks[&task].status, TaskStatus::Failed);
        assert!(s.slashed.contains(&task));
        assert!(s.check_conservation());

        // Re-resolving is a no-op (no double slash).
        let again = resolve_fraud(&s, task, None, &params);
        assert_eq!(again.supply.burned, s.supply.burned);
    }

    #[test]
    fn challenge_fraud_rewards_challenger_and_returns_bond() {
        let params = Params::devnet();
        let (s, task, _req, prov, chal) = settled_with_open_challenge();
        let bond = params.bond_min(4 * UCU_SCALE);
        assert_eq!(s.accounts[&chal].bonded, bond);

        let s = resolve_challenge(&s, task, true, &params);

        // Provider slashed; challenger made whole on bond and net-positive on reward.
        assert_eq!(s.standing_of(&prov), 0);
        assert_eq!(s.accounts[&chal].bonded, 0);
        assert!(s.accounts[&chal].balance > 100 * UCU_SCALE - bond);
        assert!(!s.pending_challenges.contains_key(&task));
        assert!(s.check_conservation());
    }

    #[test]
    fn failed_challenge_forfeits_bond() {
        let params = Params::devnet();
        let (s, task, _req, prov, chal) = settled_with_open_challenge();
        let bond = params.bond_min(4 * UCU_SCALE);
        let burned_before = s.supply.burned;

        let s = resolve_challenge(&s, task, false, &params);

        // Bond forfeited (burned); provider untouched.
        assert_eq!(s.accounts[&chal].bonded, 0);
        assert_eq!(s.accounts[&chal].balance, 100 * UCU_SCALE - bond);
        assert_eq!(s.supply.burned, burned_before + bond);
        assert_ne!(s.standing_of(&prov), 0); // not slashed
        assert!(s.check_conservation());
    }

    #[test]
    fn bond_releases_after_clawback_window() {
        let params = Params::devnet();
        let (s, task, _req, prov) = happy_path(None);
        let stake = s.accounts[&prov].bonded;
        assert!(stake > 0);
        let balance_before = s.accounts[&prov].balance;
        let receipt_before = s.receipts[&task].clone();

        // Advance to the end of the clawback window.
        let s = advance_to_epoch(&s, params.clawback_epochs, &params);

        // Stake returned to spendable balance; recorded as released.
        assert_eq!(s.accounts[&prov].bonded, 0);
        assert_eq!(s.accounts[&prov].balance, balance_before + stake);
        assert!(s.released.contains(&task));
        // Finality: the settled Receipt is never rewritten (I-ECON-FINAL).
        assert_eq!(s.receipts[&task], receipt_before);
        assert_eq!(s.tasks[&task].status, TaskStatus::Settled);
        assert!(s.check_conservation());
    }

    #[test]
    fn slashed_task_does_not_release_bond() {
        let params = Params::devnet();
        let (s, task, _req, _prov) = happy_path(None);
        let s = resolve_fraud(&s, task, None, &params);
        // Far past the window; a slashed task's stake is consumed, not released.
        let s = advance_to_epoch(&s, params.clawback_epochs + 1, &params);
        assert!(!s.released.contains(&task));
        assert!(s.check_conservation());
    }

    #[test]
    fn settled_receipt_is_immutable_through_reversal() {
        let params = Params::devnet();
        let (s, task, _req, _prov) = happy_path(None);
        let receipt_before = s.receipts[&task].clone();
        // Economic reversal via stake marks the task Failed but never edits the Receipt.
        let s = resolve_fraud(&s, task, None, &params);
        assert_eq!(s.receipts[&task], receipt_before);
        assert_eq!(s.tasks[&task].status, TaskStatus::Failed);
    }

    #[test]
    fn challenge_outside_window_is_rejected() {
        let params = Params::devnet();
        let (mut s, task, _, _) = happy_path(None);
        // Push the epoch past the clawback window.
        s.epoch = params.clawback_epochs + 1;
        let chal = keys::identity(&seed(3));
        s.accounts.insert(
            chal,
            Account {
                id: chal,
                balance: 100 * UCU_SCALE,
                escrowed: 0,
                bonded: 0,
            },
        );
        s.supply.minted += 100 * UCU_SCALE;
        let bond = params.bond_min(4 * UCU_SCALE);
        let res = apply(
            &s,
            &SignedTx::sign(&seed(3), Tx::Challenge { task, bond }, 0),
            &params,
        );
        assert_eq!(res, Err(Reject::NotInClawbackWindow));
    }
}
