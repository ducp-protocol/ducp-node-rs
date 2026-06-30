//! # ducp-types
//!
//! Canonical DUCP data model shared by every conforming node — identifiers, tasks,
//! on-ledger records, transactions, blocks, and the **Quant (ℚ)** efficiency
//! observable. Field shapes and encodings follow the reference-node binding specification
//! ([`spec/bindings/01`](https://github.com/ducp-protocol/spec)) and
//! [`spec/09`](https://github.com/ducp-protocol/spec) (DP-0001).
//!
//! ## Encoding & hashing (spec/bindings/01 §1)
//! - **Canonical bytes**: `borsh`, fields in declaration order; no floats in any
//!   hashed structure (the ℚ types are integer-only by construction).
//! - **Hash**: BLAKE3-256 over canonical bytes ([`hash_canonical`]).
//! - **Identity/Signature**: Ed25519 (see [`keys`]).
//! - **Amounts**: integer base units, `1 𝕌 = 10^9` ([`UCU_SCALE`]).
//! - **Wire form** (JSON-RPC): binary as hex strings, amounts as decimal strings
//!   (see the `serde(with = ...)` field attributes).
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//! Status: Reference implementation for DUCP-SPEC v0.2.0.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ============================ Identifiers (01 §2) ============================

pub type Hash = [u8; 32];
pub type Identity = [u8; 32]; // Ed25519 public key (reference-node binding)
pub type Signature = [u8; 64]; // Ed25519 signature
pub type ContentId = Hash; // hash of an off-ledger payload
pub type TaskId = Hash; // = hash(canonical(TaskBody))
pub type TxId = Hash; // = hash(canonical(SignedTx))
pub type Ucu = u128; // base units; 1 𝕌 = 1_000_000_000
pub type Sp = i128; // Standing points, base scale
pub type Epoch = u64;
pub type BenchmarkVersion = u32;

/// Unit precision: `1 𝕌 = 10^UCU_DECIMALS` base units (P0-provisional).
pub const UCU_DECIMALS: u32 = 9;
/// `10^UCU_DECIMALS` — the number of base units in one 𝕌.
pub const UCU_SCALE: Ucu = 1_000_000_000;
/// Fixed-point scale for ℚ: one ℚ = `MICRO_Q_SCALE` micro-ℚ.
pub const MICRO_Q_SCALE: u64 = 1_000_000;

// ============================ Hashing (01 §1) ===============================

/// BLAKE3-256 of raw bytes — the content hash of a payload (`ContentId`).
pub fn hash_bytes(bytes: &[u8]) -> Hash {
    *blake3::hash(bytes).as_bytes()
}

/// BLAKE3-256 of a value's canonical (borsh) encoding.
pub fn hash_canonical<T: BorshSerialize>(value: &T) -> Hash {
    hash_bytes(&canonical_bytes(value))
}

/// The canonical byte encoding of a value (borsh, declaration order).
pub fn canonical_bytes<T: BorshSerialize>(value: &T) -> Vec<u8> {
    borsh::to_vec(value).expect("canonical borsh encoding is infallible for our types")
}

/// Content-address a payload: `ContentId = BLAKE3(payload)`.
pub fn content_id(payload: &[u8]) -> ContentId {
    hash_bytes(payload)
}

// ===================== Errors (transition rejections) =======================

/// Why a transaction or task action was rejected. Stable variants map to
/// JSON-RPC error codes at the node boundary (05 §3).
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
    thiserror::Error,
)]
pub enum Reject {
    #[error("bad signature")]
    BadSignature,
    #[error("bad nonce")]
    BadNonce,
    #[error("unknown account")]
    UnknownAccount,
    #[error("unknown task")]
    UnknownTask,
    #[error("insufficient balance")]
    InsufficientBalance,
    #[error("task not in required status")]
    BadStatus,
    #[error("deadline passed")]
    DeadlinePassed,
    #[error("task already claimed")]
    AlreadyClaimed,
    #[error("wrong provider")]
    WrongProvider,
    #[error("ucu_count exceeds declared limit")]
    UcuExceedsLimit,
    #[error("benchmark mismatch")]
    BenchmarkMismatch,
    #[error("not within clawback window")]
    NotInClawbackWindow,
    #[error("challenge bond below minimum")]
    BondTooSmall,
    #[error("unsupported wasm feature")]
    UnsupportedFeature,
    #[error("ledger conservation violated")]
    ConservationViolated,
    #[error("invalid transaction")]
    Invalid,
}

// ============================ Tasks (01 §3) =================================

/// Intermediate representation. This binding has exactly one IR.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum IrId {
    Wasm,
}

/// Declared resource caps for a task.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct Limits {
    #[serde(with = "wire::dec_u128")]
    pub max_ucu: Ucu,
    pub max_memory_bytes: u64,
}

/// Verification tier, assigned by the DVM at submit — never chosen
/// (`I-VERIFY-NOCHOICE`). This binding assigns `SampledReexec` to every task.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum VerificationTier {
    SampledReexec,
    Tee,
    Zk,
}

/// What happens to a task that fails execution (06).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    AutoRequeue,
    ReturnOnFailure,
    RetryThenReturn { retries: u8 },
}

/// What a Requester asks for. Hashed → [`TaskId`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct TaskBody {
    pub ir: IrId,
    #[serde(with = "wire::bytes32")]
    pub program: ContentId,
    #[serde(with = "wire::bytes32")]
    pub input: ContentId,
    pub limits: Limits,
    pub tier: VerificationTier,
    pub benchmark: BenchmarkVersion,
    pub deadline: Epoch,
    pub failure_policy: FailurePolicy,
    pub nonce: u64,
}

impl TaskBody {
    /// `TaskId = hash(canonical(TaskBody))`.
    pub fn task_id(&self) -> TaskId {
        hash_canonical(self)
    }
}

// ========================= On-ledger records (01 §4) ========================

/// Lifecycle status of a task on the ledger.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Submitted,
    Matched,
    Executing,
    Verified,
    Settled,
    Failed,
}

/// Created at Submit. The Requester escrows `max_ucu + fee`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Submission {
    #[serde(with = "wire::bytes32")]
    pub task: TaskId,
    #[serde(with = "wire::bytes32")]
    pub requester: Identity,
    #[serde(with = "wire::dec_u128")]
    pub ucu_count: Ucu, // protocol-derived (I-UNIT-DERIVED); 0 until proof
    #[serde(with = "wire::dec_u128")]
    pub fee: Ucu,
    pub status: TaskStatus,
    #[serde(with = "wire::opt_bytes32")]
    pub provider: Option<Identity>,
    #[serde(with = "wire::dec_u128")]
    pub claim_stake: Ucu, // Standing-discounted (set at Match)
}

/// Tier-specific evidence carried by a [`ComputeProof`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "snake_case")]
pub enum TierData {
    SampledReexec, // binding: determinism enables exact re-check
    Tee {
        #[serde(with = "wire::bytes_vec")]
        attestation: Vec<u8>,
    },
    Zk {
        #[serde(with = "wire::bytes_vec")]
        proof: Vec<u8>,
    },
}

/// The Provider's evidence for a settled task (01 §4).
///
/// The optional [`power_seal`](ComputeProof::power_seal) is the energy attestation
/// introduced by **DP-0001**: absent by default, it never affects `ucu_count`,
/// minting, or proof validity (`I-Q-REWARDNEUTRAL`). When absent, the task's ℚ is
/// `None` (`I-Q-NULL`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct ComputeProof {
    #[serde(with = "wire::bytes32")]
    pub task: TaskId,
    #[serde(with = "wire::bytes32")]
    pub provider: Identity,
    #[serde(with = "wire::bytes32")]
    pub output: ContentId,
    #[serde(with = "wire::bytes32")]
    pub result_hash: Hash,
    #[serde(with = "wire::dec_u128")]
    pub ucu_count: Ucu,
    pub benchmark: BenchmarkVersion,
    pub tier_data: TierData,
    /// Optional, reward-neutral energy attestation (DP-0001, spec/09). `None` in P0.
    pub power_seal: Option<PowerSeal>,
}

/// Final settlement effect (recorded at Settle; immutable, `I-ECON-FINAL`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Receipt {
    #[serde(with = "wire::bytes32")]
    pub task: TaskId,
    #[serde(with = "wire::dec_u128")]
    pub paid_to_provider: Ucu,
    #[serde(with = "wire::dec_u128")]
    pub work_issuance: Ucu,
    #[serde(with = "wire::dec_u128")]
    pub validator_fee: Ucu,
    #[serde(with = "wire::dec_i128")]
    pub standing_delta: Sp,
    pub settled_epoch: Epoch,
    pub clawback_until: Epoch,
}

// ===================== Accounts & Standing (01 §5) =========================

/// A spendable-𝕌 account.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct Account {
    #[serde(with = "wire::bytes32")]
    pub id: Identity,
    #[serde(with = "wire::dec_u128")]
    pub balance: Ucu, // spendable 𝕌
    #[serde(with = "wire::dec_u128")]
    pub escrowed: Ucu, // locked in open tasks (Requester)
    #[serde(with = "wire::dec_u128")]
    pub bonded: Ucu, // stake locked in clawback windows (Provider)
}

impl Account {
    /// A fresh, empty account for `id`.
    pub fn new(id: Identity) -> Self {
        Account {
            id,
            balance: 0,
            escrowed: 0,
            bonded: 0,
        }
    }
}

/// Separate, non-spendable reputation ledger (`I-STAND-NOTMONEY`). There MUST be
/// no operation converting `sp ↔ balance` (`I-STAND-NOXFER`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub struct StandingRecord {
    #[serde(with = "wire::bytes32")]
    pub id: Identity,
    #[serde(with = "wire::dec_i128")]
    pub sp: Sp,
    pub last_decay_epoch: Epoch,
    pub strikes: u32,
}

impl StandingRecord {
    /// A fresh Standing record at zero.
    pub fn new(id: Identity, epoch: Epoch) -> Self {
        StandingRecord {
            id,
            sp: 0,
            last_decay_epoch: epoch,
            strikes: 0,
        }
    }
}

// ====================== The ℚ observable (DP-0001, 09) =====================

/// Strength of the power-cap attestation behind a [`PowerSeal`] (spec/09 §5).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
pub enum SealGrade {
    /// Self-attested static power cap. Available today; weakest evidence.
    #[serde(rename = "S0")]
    S0Identity,
    /// Out-of-band, root-of-trust–signed cap or meter (e.g. BMC / smart PDU).
    #[serde(rename = "S1")]
    S1Witnessed,
    /// Vendor-locked, signed on-die power register. Strongest; not yet available.
    #[serde(rename = "S2")]
    S2Locked,
}

/// Where energy was bounded/measured (spec/09 §5.2). The protocol never fixes a
/// single boundary; it records the declared one so ℚ is compared only within an
/// identical `(grade, boundary)` (`I-Q-COMPARE`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Boundary {
    Chip,
    Node,
    Facility,
}

/// A recorded ℚ value as fixed-point **micro-ℚ** (ℚ × 1_000_000).
///
/// Integer by construction: no floats appear in any hashed structure
/// (spec/bindings/01 §1). ℚ = 1.0 (`micro_q == 1_000_000`) is frontier-grade;
/// below 1.0 is behind the frontier, above 1.0 is ahead of it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct Quant {
    pub micro_q: u64,
}

impl Quant {
    /// Frontier-grade efficiency, ℚ = 1.0.
    pub const ONE: Quant = Quant {
        micro_q: MICRO_Q_SCALE,
    };

    /// Construct from micro-ℚ.
    pub const fn from_micro(micro_q: u64) -> Self {
        Quant { micro_q }
    }
}

/// Optional energy attestation on a [`ComputeProof`] (spec/09 §3).
///
/// It attests *configuration* (a static power cap), not data-dependent telemetry,
/// so it is side-channel-safe and signable by existing roots of trust. All fields
/// are integers — no floats in hashed data. The recorded ℚ is the *Sealed* lower
/// bound `≥ (C · E_baseline · T_std) / (power_cap · window · T_max)` (spec/09 §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct PowerSeal {
    pub seal_grade: SealGrade,
    pub boundary: Boundary,
    pub power_cap_milliwatts: u64,
    pub window_millis: u64,
    pub t_max_millikelvin: u64,
    /// Evidence chaining the seal to a hardware root of trust and to the Task Hash;
    /// bulky evidence lives off-ledger, referenced by content id.
    #[serde(with = "wire::bytes32")]
    pub attestation_evidence: ContentId,
    /// Benchmark epoch supplying `E_baseline × T_std` used to compute ℚ.
    pub benchmark: BenchmarkVersion,
}

/// One entry of the on-chain **ℚ-ledger** (spec/09 §7): the `(𝕌, ℚ)` pair recorded
/// for every settled task. `q` is `None` wherever energy was not validly attested
/// (`I-Q-NULL`); recording it never affects settlement (`I-Q-REWARDNEUTRAL`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct QLedgerEntry {
    #[serde(with = "wire::bytes32")]
    pub task: TaskId,
    #[serde(with = "wire::dec_u128")]
    pub ucu: Ucu,
    pub q: Option<Quant>,
    pub seal_grade: Option<SealGrade>,
    pub boundary: Option<Boundary>,
    pub benchmark: BenchmarkVersion,
}

impl QLedgerEntry {
    /// A reward-neutral entry with no energy attestation — the binding default for
    /// every task: 𝕌 recorded, ℚ unmeasured (`I-Q-NULL`).
    pub fn unmeasured(task: TaskId, ucu: Ucu, benchmark: BenchmarkVersion) -> Self {
        QLedgerEntry {
            task,
            ucu,
            q: None,
            seal_grade: None,
            boundary: None,
            benchmark,
        }
    }
}

// ======================== Transactions & blocks (01 §6) ====================

/// A state-changing operation. Authored and signed in a [`SignedTx`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tx {
    SubmitTask(TaskBody),
    ClaimTask {
        #[serde(with = "wire::bytes32")]
        task: TaskId,
    },
    SubmitProof(ComputeProof),
    Challenge {
        #[serde(with = "wire::bytes32")]
        task: TaskId,
        #[serde(with = "wire::dec_u128")]
        bond: Ucu,
    },
    Transfer {
        #[serde(with = "wire::bytes32")]
        to: Identity,
        #[serde(with = "wire::dec_u128")]
        amount: Ucu,
    },
}

/// A signed transaction. The signature covers `(author ‖ canonical(tx) ‖ nonce)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct SignedTx {
    #[serde(with = "wire::bytes32")]
    pub author: Identity,
    pub tx: Tx,
    pub nonce: u64,
    #[serde(with = "wire::bytes64")]
    pub sig: Signature,
}

impl SignedTx {
    /// The exact bytes the signature is computed over: `author ‖ canonical(tx) ‖ nonce_le`.
    pub fn signing_payload(author: &Identity, tx: &Tx, nonce: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(author);
        BorshSerialize::serialize(tx, &mut buf).expect("borsh into Vec is infallible");
        buf.extend_from_slice(&nonce.to_le_bytes());
        buf
    }

    /// Construct and sign a transaction with the given Ed25519 signing seed.
    pub fn sign(seed: &[u8; 32], tx: Tx, nonce: u64) -> Self {
        let author = keys::identity(seed);
        let payload = Self::signing_payload(&author, &tx, nonce);
        let sig = keys::sign(seed, &payload);
        SignedTx {
            author,
            tx,
            nonce,
            sig,
        }
    }

    /// Verify the signature binds `author` to `(tx, nonce)`.
    pub fn verify_sig(&self) -> bool {
        let payload = Self::signing_payload(&self.author, &self.tx, self.nonce);
        keys::verify(&self.author, &payload, &self.sig)
    }

    /// `TxId = hash(canonical(SignedTx))`.
    pub fn tx_id(&self) -> TxId {
        hash_canonical(self)
    }
}

/// A block of ordered transactions and the resulting state commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Block {
    pub height: u64,
    #[serde(with = "wire::bytes32")]
    pub parent: Hash,
    pub epoch: Epoch,
    #[serde(with = "wire::vec_bytes32")]
    pub txs: Vec<TxId>, // ordered by the sequencer (consensus, 04)
    #[serde(with = "wire::bytes32")]
    pub state_root: Hash, // commitment to ledger state after applying txs
    #[serde(with = "wire::bytes32")]
    pub proposer: Identity, // P0: the single sequencer
}

impl Block {
    /// `hash(canonical(Block))` — the block's identity.
    pub fn block_hash(&self) -> Hash {
        hash_canonical(self)
    }
}

// ============================ Ed25519 keys =================================

/// Ed25519 signing/verification helpers. Keys are raw byte arrays in the data
/// model (`Identity = [u8; 32]`, `Signature = [u8; 64]`); this module is the only
/// place the curve library is touched.
pub mod keys {
    use super::{Identity, Signature};
    use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};

    /// Derive the public [`Identity`] from a 32-byte signing seed.
    pub fn identity(seed: &[u8; 32]) -> Identity {
        SigningKey::from_bytes(seed).verifying_key().to_bytes()
    }

    /// Sign a message with a 32-byte signing seed.
    pub fn sign(seed: &[u8; 32], message: &[u8]) -> Signature {
        SigningKey::from_bytes(seed).sign(message).to_bytes()
    }

    /// Verify `signature` over `message` against `identity`. Returns `false` on any
    /// malformed key/signature rather than panicking.
    pub fn verify(identity: &Identity, message: &[u8], signature: &Signature) -> bool {
        let Ok(vk) = VerifyingKey::from_bytes(identity) else {
            return false;
        };
        let sig = ed25519_dalek::Signature::from_bytes(signature);
        vk.verify(message, &sig).is_ok()
    }

    /// Generate a fresh random 32-byte signing seed from the OS CSPRNG.
    pub fn generate_seed() -> [u8; 32] {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).expect("OS RNG available");
        seed
    }
}

// ===================== Wire (JSON-RPC) serde helpers =======================

/// `serde(with = ...)` modules for the JSON-RPC wire form: binary as hex strings,
/// 128-bit amounts as decimal strings. These affect **only** JSON; canonical
/// (borsh) bytes are independent.
mod wire {
    use serde::{Deserialize, Deserializer, Serializer};

    fn from_hex<const N: usize, E: serde::de::Error>(s: &str) -> Result<[u8; N], E> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        let v = hex::decode(s).map_err(E::custom)?;
        v.try_into()
            .map_err(|_| E::custom(format!("expected {N} bytes")))
    }

    pub mod bytes32 {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&hex::encode(v))
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
            let s = String::deserialize(d)?;
            from_hex::<32, _>(&s)
        }
    }

    pub mod bytes64 {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&hex::encode(v))
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
            let s = String::deserialize(d)?;
            from_hex::<64, _>(&s)
        }
    }

    pub mod opt_bytes32 {
        use super::*;
        pub fn serialize<S: Serializer>(v: &Option<[u8; 32]>, s: S) -> Result<S::Ok, S::Error> {
            match v {
                Some(b) => s.serialize_some(&hex::encode(b)),
                None => s.serialize_none(),
            }
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 32]>, D::Error> {
            let o: Option<String> = Option::deserialize(d)?;
            match o {
                None => Ok(None),
                Some(s) => Ok(Some(from_hex::<32, _>(&s)?)),
            }
        }
    }

    pub mod vec_bytes32 {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[[u8; 32]], s: S) -> Result<S::Ok, S::Error> {
            let hexed: Vec<String> = v.iter().map(hex::encode).collect();
            serde::Serialize::serialize(&hexed, s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[u8; 32]>, D::Error> {
            let items: Vec<String> = Vec::deserialize(d)?;
            items.iter().map(|s| from_hex::<32, _>(s)).collect()
        }
    }

    pub mod bytes_vec {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&hex::encode(v))
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
            let s = String::deserialize(d)?;
            let s = s.strip_prefix("0x").unwrap_or(&s);
            hex::decode(s).map_err(serde::de::Error::custom)
        }
    }

    pub mod dec_u128 {
        use super::*;
        pub fn serialize<S: Serializer>(v: &u128, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&v.to_string())
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
            let s = String::deserialize(d)?;
            s.parse().map_err(serde::de::Error::custom)
        }
    }

    pub mod dec_i128 {
        use super::*;
        pub fn serialize<S: Serializer>(v: &i128, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&v.to_string())
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i128, D::Error> {
            let s = String::deserialize(d)?;
            s.parse().map_err(serde::de::Error::custom)
        }
    }
}

// ================================ Tests ====================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task_body() -> TaskBody {
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

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }

    #[test]
    fn borsh_roundtrip_taskbody() {
        let body = sample_task_body();
        let bytes = canonical_bytes(&body);
        let back: TaskBody = borsh::from_slice(&bytes).unwrap();
        assert_eq!(body, back);
    }

    #[test]
    fn task_id_is_hash_of_canonical() {
        let body = sample_task_body();
        assert_eq!(body.task_id(), hash_bytes(&canonical_bytes(&body)));
    }

    #[test]
    fn json_roundtrip_uses_hex_and_decimal_strings() {
        let body = sample_task_body();
        let json = serde_json::to_string(&body).unwrap();
        // hex for content ids, decimal string for amounts.
        assert!(json.contains(&"11".repeat(32)));
        assert!(json.contains("10000000000")); // 10 * 1e9 as a string
        let back: TaskBody = serde_json::from_str(&json).unwrap();
        assert_eq!(body, back);
    }

    #[test]
    fn signing_and_verification_roundtrip() {
        let seed = [3u8; 32];
        let tx = Tx::Transfer {
            to: [9u8; 32],
            amount: 5 * UCU_SCALE,
        };
        let signed = SignedTx::sign(&seed, tx, 1);
        assert_eq!(signed.author, keys::identity(&seed));
        assert!(signed.verify_sig());

        // Tamper: flip the nonce → signature no longer verifies.
        let mut bad = signed.clone();
        bad.nonce = 2;
        assert!(!bad.verify_sig());
    }

    #[test]
    fn tx_id_changes_with_content() {
        let a = SignedTx::sign(
            &[1u8; 32],
            Tx::Transfer {
                to: [2u8; 32],
                amount: 1,
            },
            0,
        );
        let b = SignedTx::sign(
            &[1u8; 32],
            Tx::Transfer {
                to: [2u8; 32],
                amount: 2,
            },
            0,
        );
        assert_ne!(a.tx_id(), b.tx_id());
    }

    #[test]
    fn q_unmeasured_is_reward_neutral_null() {
        let e = QLedgerEntry::unmeasured([1u8; 32], 4 * UCU_SCALE, 0);
        assert_eq!(e.q, None);
        assert_eq!(e.seal_grade, None);
        assert_eq!(e.ucu, 4 * UCU_SCALE);
    }

    #[test]
    fn quant_one_is_frontier_grade() {
        assert_eq!(Quant::ONE.micro_q, MICRO_Q_SCALE);
    }

    #[test]
    fn seal_grade_serde_uses_short_names() {
        assert_eq!(
            serde_json::to_string(&SealGrade::S0Identity).unwrap(),
            "\"S0\""
        );
        assert_eq!(serde_json::to_string(&Boundary::Chip).unwrap(), "\"chip\"");
    }

    #[test]
    fn no_power_seal_means_proof_still_well_formed() {
        let proof = ComputeProof {
            task: [0u8; 32],
            provider: [0u8; 32],
            output: [0u8; 32],
            result_hash: [0u8; 32],
            ucu_count: 4 * UCU_SCALE,
            benchmark: 0,
            tier_data: TierData::SampledReexec,
            power_seal: None,
        };
        let bytes = canonical_bytes(&proof);
        let back: ComputeProof = borsh::from_slice(&bytes).unwrap();
        assert_eq!(proof, back);
        assert!(proof.power_seal.is_none());
    }
}
