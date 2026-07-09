//! WS6 (FINISH_INTERPRETER Phase 13) — the browser WASM-JIT tier.
//!
//! The native copy-and-patch JIT (`logicaffeine_forge`) patches x86 stencils into
//! executable memory — impossible in a browser, where there is no executable memory to
//! patch and only WebAssembly bytecode runs. The only path to JIT-level speed in WASM is a
//! *second code generator* that emits a fresh WebAssembly module per hot region and
//! instantiates it via the host's `WebAssembly.instantiate`. The byte emitter is
//! [`func`](crate::vm::wasm::func); this module is the *tier* around it — the tier-up
//! bookkeeping plus the host seam that instantiates and calls the emitted modules.
//!
//! It lives in the VM crate (not `logicaffeine_forge`) because forge is
//! `#![cfg(not(target_arch = "wasm32"))]` — it cannot build for wasm32 — whereas this
//! backend must build for and run on wasm32. The native 2.55×-C x86 JIT is untouched.

use super::func::compile_function_to_wasm;
use crate::vm::instruction::CompiledProgram;

/// The runtime WASM-JIT tier: per-function call counters plus a cache of compiled +
/// instantiated modules. When a function crosses the hot threshold it is lowered to WASM and
/// instantiated; subsequent calls dispatch to the compiled module. Functions the codegen
/// declines are remembered as `Ineligible` and stay on the bytecode tier. The VM's
/// `Op::Call` consults this only under `#[cfg(feature = "wasm-jit")]`, so the default build
/// never carries it.
///
/// The host differs by target — and this is the whole point of the WASM backend:
/// - **native**: the pure-Rust [`wasmi`] interpreter. This is also the codegen oracle the
///   differential tests cross-check the emitter against.
/// - **wasm32**: the platform's *real* `WebAssembly` (V8 in the browser / node) via
///   `js_sys::WebAssembly`. This is the production tier — a hot function tiers up to a
///   freshly-compiled native WebAssembly module the host JITs, exactly as the spec requires.
///
/// `on_call` and the tier-up bookkeeping are target-independent; only [`instantiate`] and
/// [`ReadyModule::call`] (the host seam) are `#[cfg]`-split.
pub struct WasmTier {
    entries: std::collections::HashMap<u16, TierEntry>,
    threshold: u32,
    hits: u64,
}

enum TierEntry {
    Pending(u32),
    Ready(ReadyModule),
    Ineligible,
}

/// A compiled + instantiated module ready to call. Native holds the `wasmi` store + export;
/// wasm32 holds the host `WebAssembly.Instance`'s exported function.
#[cfg(not(target_arch = "wasm32"))]
struct ReadyModule {
    store: wasmi::Store<()>,
    func: wasmi::Func,
}

#[cfg(target_arch = "wasm32")]
struct ReadyModule {
    func: js_sys::Function,
}

impl ReadyModule {
    /// Call the module's `f` with i64 args, returning its i64 result.
    #[cfg(not(target_arch = "wasm32"))]
    fn call(&mut self, args: &[i64]) -> Option<i64> {
        let argv: Vec<wasmi::Value> = args.iter().map(|&a| wasmi::Value::I64(a)).collect();
        let mut results = [wasmi::Value::I64(0)];
        self.func.call(&mut self.store, &argv, &mut results).ok()?;
        match results[0] {
            wasmi::Value::I64(v) => Some(v),
            _ => None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn call(&mut self, args: &[i64]) -> Option<i64> {
        call_host_func(&self.func, args)
    }
}

impl WasmTier {
    /// A tier that compiles a function after `threshold` calls (clamped to ≥1).
    pub fn new(threshold: u32) -> Self {
        WasmTier {
            entries: std::collections::HashMap::new(),
            threshold: threshold.max(1),
            hits: 0,
        }
    }

    /// How many calls have dispatched to a compiled WASM module — a diagnostic/test hook
    /// proving the tier actually fired.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Run `program.functions[func](args)` on the WASM-JIT tier, or `None` to fall back to
    /// the bytecode tier (not yet hot, or ineligible). A `Some` result is the emitted
    /// module's output — cross-checked against the VM by the differential tests.
    pub fn on_call(&mut self, program: &CompiledProgram, func: u16, args: &[i64]) -> Option<i64> {
        self.entries.entry(func).or_insert(TierEntry::Pending(0));
        // Count the call; cross the threshold ⇒ compile + instantiate (or mark ineligible).
        if let Some(TierEntry::Pending(count)) = self.entries.get_mut(&func) {
            *count += 1;
            if *count < self.threshold {
                return None;
            }
        }
        if matches!(self.entries.get(&func), Some(TierEntry::Pending(_))) {
            match instantiate(program, func) {
                Some(m) => {
                    self.entries.insert(func, TierEntry::Ready(m));
                }
                None => {
                    self.entries.insert(func, TierEntry::Ineligible);
                    return None;
                }
            }
        }
        // Dispatch to the compiled module.
        if let Some(TierEntry::Ready(m)) = self.entries.get_mut(&func) {
            if let Some(v) = m.call(args) {
                self.hits += 1;
                return Some(v);
            }
        }
        None
    }
}

/// Lower function `func` to WASM and instantiate it through the native `wasmi` host.
#[cfg(not(target_arch = "wasm32"))]
fn instantiate(program: &CompiledProgram, func: u16) -> Option<ReadyModule> {
    let bytes = compile_function_to_wasm(program, func as usize)?;
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, &bytes[..]).ok()?;
    let mut store = wasmi::Store::new(&engine, ());
    let instance = wasmi::Linker::<()>::new(&engine)
        .instantiate(&mut store, &module)
        .ok()?
        .start(&mut store)
        .ok()?;
    let f = instance.get_func(&store, "f")?;
    Some(ReadyModule { store, func: f })
}

/// Lower function `func` to WASM and instantiate it through the host's real `WebAssembly`.
#[cfg(target_arch = "wasm32")]
fn instantiate(program: &CompiledProgram, func: u16) -> Option<ReadyModule> {
    let bytes = compile_function_to_wasm(program, func as usize)?;
    Some(ReadyModule { func: instantiate_on_host(&bytes)? })
}

/// Compile + instantiate raw WASM bytes through the host's `WebAssembly` (V8 in the browser
/// / node), returning the module's exported `f`. The constructors `new WebAssembly.Module`
/// and `new WebAssembly.Instance` are synchronous (unlike `WebAssembly.instantiate`), so
/// tier-up stays a plain synchronous step inside the VM's `Op::Call`. wasm32 only.
#[cfg(target_arch = "wasm32")]
fn instantiate_on_host(bytes: &[u8]) -> Option<js_sys::Function> {
    use wasm_bindgen::JsCast;
    let arr = js_sys::Uint8Array::from(bytes);
    let module = js_sys::WebAssembly::Module::new(arr.as_ref()).ok()?;
    let instance = js_sys::WebAssembly::Instance::new(&module, &js_sys::Object::new()).ok()?;
    js_sys::Reflect::get(instance.exports().as_ref(), &wasm_bindgen::JsValue::from_str("f"))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()
}

/// Call a host `WebAssembly` export taking/returning i64. WebAssembly i64 crosses the JS
/// boundary as `BigInt`; args are marshaled in as `BigInt`s and the `BigInt` result is read
/// back losslessly via its base-10 string (no f64 round-trip). wasm32 only.
#[cfg(target_arch = "wasm32")]
fn call_host_func(f: &js_sys::Function, args: &[i64]) -> Option<i64> {
    use wasm_bindgen::JsCast;
    let arr = js_sys::Array::new();
    for &a in args {
        arr.push(&wasm_bindgen::JsValue::from(js_sys::BigInt::from(a)));
    }
    let result = f.apply(&wasm_bindgen::JsValue::NULL, &arr).ok()?;
    let bigint = result.unchecked_into::<js_sys::BigInt>();
    let decimal = wasm_bindgen::JsValue::from(bigint.to_string(10).ok()?).as_string()?;
    decimal.parse::<i64>().ok()
}

/// Instantiate raw WASM bytes through the host's real `WebAssembly` and call export `f` with
/// i64 args — the browser-native execution primitive the WS6 browser tests use to prove the
/// emitted modules run on V8 (not just wasmi). wasm32 only.
#[cfg(target_arch = "wasm32")]
pub fn run_on_host(bytes: &[u8], args: &[i64]) -> Option<i64> {
    call_host_func(&instantiate_on_host(bytes)?, args)
}
