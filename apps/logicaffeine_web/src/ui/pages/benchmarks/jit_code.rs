//! `latest-jit-code.json`: the x86-64 machine code the LOGOS (JIT) row emitted for each benchmark during the measured
//! run, one record per compiled function or loop region, disassembled by the run's assembler. The native build compiles
//! the file in when a run has written it; the wasm build fetches `/data/latest-jit-code.json` the first time the
//! LOGOS (JIT) tab opens.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::LazyLock;
#[cfg(target_arch = "wasm32")]
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(super) struct JitCodeData {
    pub(super) benchmarks: HashMap<String, Vec<JitRegion>>,
}

/// One compiled function or loop region: its symbol (`logos::qs [jit]`, `logos::main::loop@36 [jit]`), where it was
/// installed, its size, and its disassembly at that address.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub(super) struct JitRegion {
    pub(super) name: String,
    pub(super) start: String,
    pub(super) bytes: u64,
    pub(super) asm: String,
}

impl JitRegion {
    /// The region's name without the perf-map decoration: `qs`, `main · loop at pc 36`.
    pub(super) fn title(&self) -> String {
        let bare = self
            .name
            .trim_start_matches("logos::")
            .trim_end_matches(" [jit]");
        match bare.split_once("::loop@") {
            Some((function, pc)) => format!("{function} \u{00b7} loop at pc {pc}"),
            None => bare.to_string(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
static JIT_CODE_JSON: Option<&str> = include!(concat!(env!("OUT_DIR"), "/bench_jit_code.rs"));

#[cfg(not(target_arch = "wasm32"))]
static JIT_CODE: LazyLock<Option<JitCodeData>> = LazyLock::new(|| {
    JIT_CODE_JSON.map(|json| serde_json::from_str(json).expect("latest-jit-code.json is schema 1"))
});

#[cfg(target_arch = "wasm32")]
static JIT_CODE: OnceLock<Option<JitCodeData>> = OnceLock::new();

/// The JIT's machine code for every benchmark, when a run has recorded it (and, on wasm, [`load_jit_code`] fetched it).
pub(super) fn jit_code_data() -> Option<&'static JitCodeData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        JIT_CODE.as_ref()
    }
    #[cfg(target_arch = "wasm32")]
    {
        JIT_CODE.get().and_then(Option::as_ref)
    }
}

/// Whether the fetch has finished (found or not), so the tab can tell "loading" from "none recorded".
pub(super) fn jit_code_settled() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        true
    }
    #[cfg(target_arch = "wasm32")]
    {
        JIT_CODE.get().is_some()
    }
}

/// Fetch `/data/latest-jit-code.json` once. A missing file (no run staged with one) is recorded as "none recorded".
#[cfg(target_arch = "wasm32")]
pub(super) async fn load_jit_code() {
    if JIT_CODE.get().is_some() {
        return;
    }
    let code =
        crate::ui::data_fetch::fetch_static_json::<JitCodeData>("/data/latest-jit-code.json")
            .await
            .ok();
    let _ = JIT_CODE.set(code);
}
