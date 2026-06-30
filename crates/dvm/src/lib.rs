//! # ducp-dvm
//!
//! The binding DUCP Virtual Machine: a **deterministic WebAssembly runtime**
//! ([wasmtime]) that executes a task once and derives its 𝕌 count by fuel metering
//! (spec/bindings/02).
//!
//! Determinism (`I-DVM-DET`): NaN canonicalization on; single-threaded; no ambient
//! capabilities (no clock, randomness, filesystem, or network); the only host
//! imports are the deterministic `ducp` ABI (§3); fixed memory limits. Given the
//! same `{module, input, benchmark}`, every conforming DVM MUST produce the
//! identical `output`, `result_hash`, and `ucu_count`.
//!
//! Specification: <https://github.com/ducp-protocol/spec>
//! Status: Reference implementation for DUCP-SPEC v0.2.0.

use borsh::{BorshDeserialize, BorshSerialize};
use ducp_types::{hash_bytes, Hash, IrId, Limits, Ucu, UCU_SCALE};
use wasmtime::{
    Caller, Config, Engine, Extern, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder,
    Trap,
};

/// Returns this crate's version, as declared in `Cargo.toml`.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A generous fuel ceiling for advisory metering ([`Dvm::meter`]), where no task
/// `Limits` are supplied. Large enough that real reference workloads complete.
const METER_FUEL_CEILING: u64 = 1 << 40;

// =============================== Benchmark =================================

/// The single consensus reference that fixes the scale of 𝕌 (`I-UNIT-ONEBENCH`,
/// spec/bindings/02 §5). `fuel_per_ucu` is calibrated from the canonical
/// reference workload so that workload meters to exactly one 𝕌.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Benchmark {
    pub version: u32,
    /// Content hash identifying the fuel cost model in force (provisional: the
    /// wasmtime fuel semantics version).
    pub fuel_cost_table_hash: Hash,
    /// `fuel_ref / UCU_REF`; here `UCU_REF = 1 𝕌`, so this is the reference fuel.
    pub fuel_per_ucu: u64,
    /// Nominal standard energy per 𝕌 (provisional integer unit) — the `E_baseline`
    /// of the ℚ definition (spec/09 §4). Recorded for the efficiency observable; does
    /// not affect `ucu_count` (`I-UNIT-ENERGYFREE`).
    pub e_baseline: u64,
    /// Standard temperature in millikelvin — the `T_std` of the ℚ definition
    /// (spec/09 §4).
    pub t_std_millikelvin: u64,
}

impl Benchmark {
    /// Calibrate the devnet benchmark: run the canonical reference workload and set
    /// `fuel_per_ucu` to the fuel it consumes (so the reference equals one 𝕌).
    pub fn devnet(dvm: &WasmtimeDvm) -> Benchmark {
        let fuel_ref = dvm
            .measure_fuel(&reference_module(), REFERENCE_INPUT)
            .expect("reference workload completes within the meter ceiling");
        Benchmark {
            version: 0,
            fuel_cost_table_hash: hash_bytes(FUEL_COST_MODEL_ID),
            fuel_per_ucu: fuel_ref.max(1),
            // ℚ baseline (spec/09): 13.7 pJ/𝕌 frontier energy at 300 K, in the
            // provisional integer units used by the Sealed-ℚ floor (0.1 pJ; mK).
            e_baseline: 137,
            t_std_millikelvin: 300_000,
        }
    }

    /// Convert a fuel quantity into a 𝕌 base-unit count under this benchmark
    /// (integer division, deterministic): `ucu = fuel * UCU_SCALE / fuel_per_ucu`.
    pub fn fuel_to_ucu(&self, fuel: u64) -> Ucu {
        (fuel as u128) * (UCU_SCALE) / (self.fuel_per_ucu as u128)
    }

    /// Convert a 𝕌 ceiling into a fuel ceiling: `max_fuel = max_ucu * fuel_per_ucu / UCU_SCALE`.
    pub fn ucu_to_fuel_ceiling(&self, max_ucu: Ucu) -> u64 {
        let f = max_ucu * (self.fuel_per_ucu as u128) / UCU_SCALE;
        f.min(u64::MAX as u128) as u64
    }
}

/// Identifier of the fuel cost model (content-hashed into the benchmark). Bump when
/// the metering semantics change (e.g. a wasmtime upgrade that alters fuel costs).
const FUEL_COST_MODEL_ID: &[u8] = b"ducp.fuel.wasmtime.v46.profile0";

// ============================ Execution outcome ===========================

/// Why a task execution did not complete successfully. Deterministic and part of
/// the canonical `result_hash` for failed runs (spec/bindings/02 §6).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum FailureKind {
    /// Fuel ceiling (from `Limits.max_ucu`) exhausted before completion.
    OutOfFuel,
    /// Guest called `ducp.fail(code)`.
    UserAbort(i32),
    /// A wasm trap (illegal instruction, OOB access, unreachable, …).
    Trap,
    /// The module failed to validate/compile under the binding Wasm feature set.
    InvalidModule,
    /// The module could not be instantiated (e.g. an unsatisfiable import — no
    /// ambient capabilities are provided).
    Instantiation,
    /// No `run`/`_start` entry point was exported.
    NoEntryPoint,
}

/// Terminal status of an execution.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum ExecStatus {
    Ok,
    Failure(FailureKind),
}

/// The deterministic result of running a task in the DVM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutcome {
    /// `hash(output)` on success; `hash(domain ‖ canonical(FailureKind))` on failure.
    pub result_hash: Hash,
    /// The output payload (empty on failure). Content-addressed by the caller.
    pub output: Vec<u8>,
    /// Metered work in 𝕌 base units, derived from fuel and the benchmark.
    pub ucu_count: Ucu,
    pub status: ExecStatus,
}

/// Domain separator for hashing a failure outcome's `result_hash`.
const FAIL_DOMAIN: &[u8] = b"ducp.fail.v0";

fn failure_result_hash(kind: &FailureKind) -> Hash {
    let mut buf = FAIL_DOMAIN.to_vec();
    buf.extend_from_slice(&borsh::to_vec(kind).expect("borsh kind"));
    hash_bytes(&buf)
}

// ================================ Dvm trait ===============================

/// The DUCP Virtual Machine interface (spec/bindings/02 §7). `execute` is
/// called once by the Provider; `execute`/`meter` are called by re-executors during
/// sampling/challenge. Because both are deterministic, comparison is exact.
pub trait Dvm {
    /// Validate a module against the binding Wasm feature set. A module using a
    /// forbidden feature MUST be rejected at submit (`Reject::UnsupportedFeature`)
    /// and never reach metering.
    fn validate(&self, module: &[u8]) -> Result<(), ducp_types::Reject>;

    /// Execute deterministically: identical `(module, input, benchmark)` →
    /// identical [`ExecOutcome`] on any host.
    fn execute(
        &self,
        module: &[u8],
        input: &[u8],
        limits: &Limits,
        benchmark: &Benchmark,
    ) -> ExecOutcome;

    /// Re-derive only the 𝕌 count (advisory; used at submit and by verifiers).
    fn meter(&self, module: &[u8], input: &[u8], benchmark: &Benchmark) -> Ucu;
}

// ============================== Host state ================================

struct HostState {
    input: Vec<u8>,
    output: Vec<u8>,
    fail_code: Option<i32>,
    limits: StoreLimits,
}

impl HostState {
    fn new(input: Vec<u8>, max_memory_bytes: u64) -> Self {
        let limits = StoreLimitsBuilder::new()
            .memory_size(max_memory_bytes as usize)
            .build();
        HostState {
            input,
            output: Vec::new(),
            fail_code: None,
            limits,
        }
    }
}

fn caller_memory(caller: &mut Caller<'_, HostState>) -> Option<Memory> {
    match caller.get_export("memory") {
        Some(Extern::Memory(m)) => Some(m),
        _ => None,
    }
}

// ============================== The runtime ===============================

/// A deterministic wasmtime-backed DVM. The [`Engine`] (and its `Config`) is built
/// once and reused; it carries the determinism settings.
pub struct WasmtimeDvm {
    engine: Engine,
}

impl Default for WasmtimeDvm {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmtimeDvm {
    /// Build a DVM with the binding determinism configuration.
    pub fn new() -> Self {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.cranelift_nan_canonicalization(true);
        // No nondeterministic / out-of-scope proposals (spec/bindings/02 §1).
        config.wasm_simd(false);
        config.wasm_relaxed_simd(false);
        config.wasm_threads(false);
        config.wasm_reference_types(false);
        config.wasm_function_references(false);
        config.wasm_gc(false);
        // Allowed binding Wasm features.
        config.wasm_bulk_memory(true);
        config.wasm_multi_value(true);
        let engine = Engine::new(&config).expect("valid deterministic wasmtime config");
        WasmtimeDvm { engine }
    }

    fn define_abi(&self, linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
        linker.func_wrap(
            "ducp",
            "input_len",
            |caller: Caller<'_, HostState>| -> i32 { caller.data().input.len() as i32 },
        )?;
        linker.func_wrap(
            "ducp",
            "input_read",
            |mut caller: Caller<'_, HostState>, dst: i32, offset: i32, len: i32| -> i32 {
                let Some(mem) = caller_memory(&mut caller) else {
                    return 0;
                };
                let (data, state) = mem.data_and_store_mut(&mut caller);
                let (dst, offset, len) = (dst as usize, offset as usize, len as usize);
                let avail = state.input.len().saturating_sub(offset);
                let n = len.min(avail);
                match dst.checked_add(n) {
                    Some(end) if end <= data.len() => {
                        data[dst..end].copy_from_slice(&state.input[offset..offset + n]);
                        n as i32
                    }
                    _ => 0,
                }
            },
        )?;
        linker.func_wrap(
            "ducp",
            "output_write",
            |mut caller: Caller<'_, HostState>, src: i32, len: i32| {
                let Some(mem) = caller_memory(&mut caller) else {
                    return;
                };
                let (data, state) = mem.data_and_store_mut(&mut caller);
                let (src, len) = (src as usize, len as usize);
                if let Some(end) = src.checked_add(len) {
                    if end <= data.len() {
                        state.output.extend_from_slice(&data[src..end]);
                    }
                }
            },
        )?;
        linker.func_wrap(
            "ducp",
            "fail",
            |mut caller: Caller<'_, HostState>, code: i32| -> wasmtime::Result<()> {
                caller.data_mut().fail_code = Some(code);
                Err(wasmtime::Error::msg("ducp.fail"))
            },
        )?;
        Ok(())
    }

    /// Core deterministic run. Returns `(status, output, fuel_consumed)`.
    fn run(
        &self,
        module_bytes: &[u8],
        input: &[u8],
        fuel_ceiling: u64,
        max_memory_bytes: u64,
    ) -> (ExecStatus, Vec<u8>, u64) {
        let module = match Module::new(&self.engine, module_bytes) {
            Ok(m) => m,
            Err(_) => {
                return (
                    ExecStatus::Failure(FailureKind::InvalidModule),
                    Vec::new(),
                    0,
                )
            }
        };

        let mut store = Store::new(
            &self.engine,
            HostState::new(input.to_vec(), max_memory_bytes),
        );
        store.limiter(|s| &mut s.limits);
        // Fuel is enabled in Config, so set_fuel succeeds.
        let _ = store.set_fuel(fuel_ceiling);

        let mut linker = Linker::new(&self.engine);
        if self.define_abi(&mut linker).is_err() {
            return (
                ExecStatus::Failure(FailureKind::Instantiation),
                Vec::new(),
                0,
            );
        }

        let instance = match linker.instantiate(&mut store, &module) {
            Ok(i) => i,
            Err(_) => {
                let fuel = consumed(&store, fuel_ceiling);
                return (
                    ExecStatus::Failure(FailureKind::Instantiation),
                    Vec::new(),
                    fuel,
                );
            }
        };

        let entry = instance
            .get_typed_func::<(), ()>(&mut store, "run")
            .or_else(|_| instance.get_typed_func::<(), ()>(&mut store, "_start"));
        let entry = match entry {
            Ok(f) => f,
            Err(_) => {
                let fuel = consumed(&store, fuel_ceiling);
                return (
                    ExecStatus::Failure(FailureKind::NoEntryPoint),
                    Vec::new(),
                    fuel,
                );
            }
        };

        let call = entry.call(&mut store, ());
        let fuel = consumed(&store, fuel_ceiling);
        match call {
            Ok(()) => {
                let output = std::mem::take(&mut store.data_mut().output);
                (ExecStatus::Ok, output, fuel)
            }
            Err(err) => {
                let kind = if let Some(code) = store.data().fail_code {
                    FailureKind::UserAbort(code)
                } else if matches!(err.downcast_ref::<Trap>(), Some(Trap::OutOfFuel)) {
                    FailureKind::OutOfFuel
                } else {
                    FailureKind::Trap
                };
                (ExecStatus::Failure(kind), Vec::new(), fuel)
            }
        }
    }

    /// Measure the raw fuel a module consumes (benchmark-independent; used to
    /// calibrate [`Benchmark::devnet`]). `None` if it does not complete.
    pub fn measure_fuel(&self, module_bytes: &[u8], input: &[u8]) -> Option<u64> {
        let (status, _out, fuel) =
            self.run(module_bytes, input, METER_FUEL_CEILING, DEFAULT_MAX_MEM);
        match status {
            ExecStatus::Ok => Some(fuel),
            _ => None,
        }
    }
}

/// Registry mapping an IR to its deterministic executor. This binding has exactly one
/// IR (WebAssembly); RISC-V / tensor IRs are added as additional entries here with
/// no change to the task lifecycle (the `Ir` registry seam, spec/implementation
/// README).
pub struct IrRegistry {
    wasm: WasmtimeDvm,
}

impl Default for IrRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl IrRegistry {
    pub fn new() -> Self {
        IrRegistry {
            wasm: WasmtimeDvm::new(),
        }
    }

    /// The executor for an IR, or `None` if the IR is unsupported in this profile.
    pub fn executor(&self, ir: IrId) -> Option<&dyn Dvm> {
        match ir {
            IrId::Wasm => Some(&self.wasm),
        }
    }
}

/// Default memory ceiling for advisory metering / calibration (16 MiB).
const DEFAULT_MAX_MEM: u64 = 16 * 1024 * 1024;

fn consumed(store: &Store<HostState>, ceiling: u64) -> u64 {
    ceiling.saturating_sub(store.get_fuel().unwrap_or(0))
}

impl Dvm for WasmtimeDvm {
    fn validate(&self, module: &[u8]) -> Result<(), ducp_types::Reject> {
        Module::new(&self.engine, module)
            .map(|_| ())
            .map_err(|_| ducp_types::Reject::UnsupportedFeature)
    }

    fn execute(
        &self,
        module: &[u8],
        input: &[u8],
        limits: &Limits,
        benchmark: &Benchmark,
    ) -> ExecOutcome {
        let ceiling = benchmark.ucu_to_fuel_ceiling(limits.max_ucu);
        let (status, output, fuel) = self.run(module, input, ceiling, limits.max_memory_bytes);
        let (result_hash, ucu_count) = match &status {
            ExecStatus::Ok => (hash_bytes(&output), benchmark.fuel_to_ucu(fuel)),
            ExecStatus::Failure(FailureKind::OutOfFuel) => {
                // Consumed the whole ceiling → the declared max_ucu (02 §6).
                (failure_result_hash(&FailureKind::OutOfFuel), limits.max_ucu)
            }
            ExecStatus::Failure(kind) => (failure_result_hash(kind), benchmark.fuel_to_ucu(fuel)),
        };
        ExecOutcome {
            result_hash,
            output,
            ucu_count,
            status,
        }
    }

    fn meter(&self, module: &[u8], input: &[u8], benchmark: &Benchmark) -> Ucu {
        let (status, _output, fuel) = self.run(module, input, METER_FUEL_CEILING, DEFAULT_MAX_MEM);
        match status {
            ExecStatus::Ok => benchmark.fuel_to_ucu(fuel),
            _ => benchmark.fuel_to_ucu(fuel),
        }
    }
}

// ===================== Canonical reference & sample modules ================

/// The canonical reference workload (WAT). A fixed, deterministic compute loop;
/// its fuel cost calibrates `fuel_per_ucu` so it equals exactly one 𝕌.
pub const REFERENCE_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "run")
    (local $i i32) (local $sum i64)
    (local.set $i (i32.const 0))
    (local.set $sum (i64.const 0))
    (block $done
      (loop $loop
        (br_if $done (i32.ge_u (local.get $i) (i32.const 100000)))
        (local.set $sum (i64.add (local.get $sum) (i64.extend_i32_u (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (i64.store (i32.const 0) (local.get $sum))
    (call $noop)
  )
  (func $noop)
)
"#;

/// Input for the reference workload (empty).
pub const REFERENCE_INPUT: &[u8] = &[];

/// Compile the canonical reference workload to wasm bytes.
pub fn reference_module() -> Vec<u8> {
    wat::parse_str(REFERENCE_WAT).expect("reference WAT is valid")
}

/// An echo workload (WAT): copies the input payload to the output verbatim. Used in
/// the metering vectors and ABI tests.
pub const ECHO_WAT: &str = r#"
(module
  (import "ducp" "input_len" (func $len (result i32)))
  (import "ducp" "input_read" (func $read (param i32 i32 i32) (result i32)))
  (import "ducp" "output_write" (func $write (param i32 i32)))
  (memory (export "memory") 1)
  (func (export "run")
    (local $n i32)
    (local.set $n (call $len))
    (drop (call $read (i32.const 0) (i32.const 0) (local.get $n)))
    (call $write (i32.const 0) (local.get $n))
  )
)
"#;

/// Compile the echo workload to wasm bytes.
pub fn echo_module() -> Vec<u8> {
    wat::parse_str(ECHO_WAT).expect("echo WAT is valid")
}

// ================================= Tests ==================================

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_ucu: Ucu) -> Limits {
        Limits {
            max_ucu,
            max_memory_bytes: DEFAULT_MAX_MEM,
        }
    }

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
    }

    #[test]
    fn reference_meters_to_exactly_one_ucu() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let ucu = dvm.meter(&reference_module(), REFERENCE_INPUT, &bench);
        assert_eq!(ucu, UCU_SCALE, "reference workload defines 1 𝕌");
    }

    #[test]
    fn execution_is_deterministic() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let m = echo_module();
        let a = dvm.execute(&m, b"hello world", &limits(UCU_SCALE), &bench);
        let b = dvm.execute(&m, b"hello world", &limits(UCU_SCALE), &bench);
        assert_eq!(a.result_hash, b.result_hash);
        assert_eq!(a.ucu_count, b.ucu_count);
        assert_eq!(a.output, b.output);
    }

    #[test]
    fn echo_output_and_result_hash() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let out = dvm.execute(&echo_module(), b"abc", &limits(UCU_SCALE), &bench);
        assert_eq!(out.status, ExecStatus::Ok);
        assert_eq!(out.output, b"abc");
        assert_eq!(out.result_hash, hash_bytes(b"abc"));
    }

    #[test]
    fn user_abort_is_deterministic_failure() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let m = wat::parse_str(
            r#"(module (import "ducp" "fail" (func $f (param i32)))
                  (func (export "run") (call $f (i32.const 7))))"#,
        )
        .unwrap();
        let out = dvm.execute(&m, b"", &limits(UCU_SCALE), &bench);
        assert_eq!(out.status, ExecStatus::Failure(FailureKind::UserAbort(7)));
        assert_eq!(out.output, Vec::<u8>::new());
        assert_eq!(
            out.result_hash,
            failure_result_hash(&FailureKind::UserAbort(7))
        );
    }

    #[test]
    fn infinite_loop_runs_out_of_fuel_at_max_ucu() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let m = wat::parse_str(r#"(module (func (export "run") (loop $l (br $l))))"#).unwrap();
        let cap = UCU_SCALE / 1000; // small ceiling
        let out = dvm.execute(&m, b"", &limits(cap), &bench);
        assert_eq!(out.status, ExecStatus::Failure(FailureKind::OutOfFuel));
        assert_eq!(out.ucu_count, cap, "OOF meters to the declared ceiling");
    }

    #[test]
    fn forbidden_simd_feature_is_rejected() {
        let dvm = WasmtimeDvm::new();
        let m = wat::parse_str(
            r#"(module (memory 1) (func (export "run") (drop (v128.load (i32.const 0)))))"#,
        )
        .unwrap();
        assert_eq!(
            dvm.validate(&m),
            Err(ducp_types::Reject::UnsupportedFeature)
        );
    }

    #[test]
    fn valid_module_passes_validation() {
        let dvm = WasmtimeDvm::new();
        assert!(dvm.validate(&echo_module()).is_ok());
        assert!(dvm.validate(&reference_module()).is_ok());
    }

    #[test]
    fn ir_registry_resolves_wasm() {
        let reg = IrRegistry::new();
        assert!(reg.executor(IrId::Wasm).is_some());
    }

    #[test]
    fn missing_entry_point_is_failure() {
        let dvm = WasmtimeDvm::new();
        let bench = Benchmark::devnet(&dvm);
        let m = wat::parse_str(r#"(module (memory (export "memory") 1))"#).unwrap();
        let out = dvm.execute(&m, b"", &limits(UCU_SCALE), &bench);
        assert_eq!(out.status, ExecStatus::Failure(FailureKind::NoEntryPoint));
    }
}
