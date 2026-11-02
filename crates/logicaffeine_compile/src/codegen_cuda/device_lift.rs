//! Tree-shaking device-lift — turn a `codegen_c` translation unit into a single-file CUDA program by
//! lifting ONLY the runtime slice the program actually references onto the device.
//!
//! `codegen_c` emits the whole ~1900-line runtime for every program; nvcc cannot compile the unused
//! bignum/`vsnprintf`/`strtod` for a float one-liner. So the CUDA tier tree-shakes the oracle (via the
//! shared [`crate::codegen_c::cshake`] reachability pass), keeping only the units reachable from the
//! user region, `__device__`-annotating them, applying device OVERRIDES for the irreducibly-host ones
//! (`logos_fmt_f64` → Dragon4), and REFUSING (typed) any reachable unit that still needs a device libc
//! we do not yet provide. `codegen_c` is untouched; the nvcc-compile lock is the safety net for any
//! missed edge.

use std::collections::BTreeSet;

use crate::codegen_c::cshake::{
    c_identifiers, macro_references, parse_c_units, reachable_units, CUnit, UnitKind,
};
use crate::codegen_c::runtime::{C_CONCURRENCY_RUNTIME, C_STEPPED_HOST, STEPPED_SCHEDULER};
use crate::codegen_c::USER_CODE_SENTINEL;

use super::device_float::LOGOS_DEVICE_FLOAT_FMT;
use super::device_string::CUDA_DEVICE_STRING_SHIM;
use super::runtime::{
    CUDA_DEVICE_ALLOC_SHIM, CUDA_DEVICE_CLOCK, CUDA_DEVICE_FAULT_SHIM, CUDA_DEVICE_INTEGRAL_ROUNDING,
    CUDA_DEVICE_INTRIN_SHIM,
    CUDA_DEVICE_PREAMBLE, CUDA_DEVICE_STEPPED_PROLOGUE, CUDA_HOST_RUN_FAILED, CUDA_HOST_RUN_SEED,
    CUDA_HOST_SEED_READER, CUDA_HOST_TEMPLATE,
};
use super::wrap::lift_user_region;

/// The names only the coroutine tier's runtime defines (`C_CONCURRENCY_RUNTIME` less the stepped scheduler's):
/// its `ucontext` stacks, trampoline and select staging, which a GPU thread cannot run. A concurrent program
/// reaches the device through the stepped tier, so a reachable unit defining one of these is refused.
fn coroutine_only_names() -> &'static BTreeSet<String> {
    static NAMES: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let stepped: BTreeSet<String> =
            parse_c_units(&format!("{C_STEPPED_HOST}{STEPPED_SCHEDULER}"))
                .into_iter()
                .flat_map(|u| u.defined)
                .collect();
        parse_c_units(C_CONCURRENCY_RUNTIME)
            .into_iter()
            .flat_map(|u| u.defined)
            .filter(|d| !stepped.contains(d))
            .collect()
    })
}

/// A concurrent program's C, around the stepped scheduler it carries in its runtime region: the runtime before
/// it, and the runtime after it (`None` when the program has no tasks, and so no scheduler).
fn split_at_scheduler(runtime: &str) -> Result<(&str, Option<&str>), &'static str> {
    match runtime.split_once(C_STEPPED_HOST) {
        None => Ok((runtime, None)),
        Some((before, rest)) => {
            let after = rest
                .strip_prefix(STEPPED_SCHEDULER)
                .ok_or("unexpected C shape (the stepped scheduler is not whole)")?;
            Ok((before, Some(after)))
        }
    }
}

/// The device replacement for a libc/compiler-builtin identifier the lifted code calls directly but
/// device code lacks — the header-carrying allocator ([`CUDA_DEVICE_ALLOC_SHIM`], so `realloc` recovers
/// the old size), the string formatters + `str*` ([`CUDA_DEVICE_STRING_SHIM`]), and the checked-overflow
/// builtins ([`CUDA_DEVICE_INTRIN_SHIM`], via `__int128`). `None` = leave the identifier untouched.
fn routed_name(word: &str) -> Option<&'static str> {
    Some(match word {
        "malloc" => "logos_malloc",
        "free" => "logos_free",
        "calloc" => "logos_calloc",
        "realloc" => "logos_realloc",
        "strdup" => "logos_strdup",
        "snprintf" => "logos_snprintf",
        "vsnprintf" => "logos_vsnprintf",
        "sprintf" => "logos_sprintf",
        "strlen" => "logos_strlen",
        "strcmp" => "logos_strcmp",
        "strncmp" => "logos_strncmp",
        "strcpy" => "logos_strcpy",
        "strncpy" => "logos_strncpy",
        "strcat" => "logos_strcat",
        "memcmp" => "logos_memcmp",
        "__builtin_mul_overflow" => "logos_mul_overflow",
        "__builtin_add_overflow" => "logos_add_overflow",
        "__builtin_sub_overflow" => "logos_sub_overflow",
        // Runtime-error path → device trap (see CUDA_DEVICE_FAULT_SHIM). Routed in the LIFTED runtime
        // only, so the host orchestrator keeps real libc. `stderr`/`stdout` become ignored sink ints;
        // the C runtime's `fflush(stdout)` before a trap (partial-output parity with the VM) is a
        // device no-op — a kernel's printf buffer is flushed by the host after the launch anyway.
        // Locked by cuda_device_fault_shim_lock: a fail-path libc name with no route here is RED.
        "exit" => "logos_exit",
        "abort" => "logos_abort",
        "fprintf" => "logos_fprintf",
        "stderr" => "logos_stderr_sink",
        "fflush" => "logos_fflush",
        "stdout" => "logos_stdout_sink",
        _ => return None,
    })
}

/// Rewrite whole-identifier calls to their device replacements ([`routed_name`]) in a device code span,
/// skipping string/char literals and comments so only real call sites are touched. Everything else is copied
/// as the text it is: the span is cut only at ASCII delimiters, so a literal or a comment that spells a
/// character past ASCII keeps its UTF-8 bytes.
fn route_alloc_to_shims(code: &str) -> String {
    let b = code.as_bytes();
    let mut out = String::with_capacity(code.len() + 64);
    let mut i = 0;
    while i < b.len() {
        let start = i;
        match b[i] {
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' && i + 1 < b.len() { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                out.push_str(&code[start..i]);
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                out.push_str(&code[start..i]);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                out.push_str(&code[start..i]);
            }
            c if c == b'_' || c.is_ascii_alphabetic() => {
                while i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphanumeric()) {
                    i += 1;
                }
                let word = &code[start..i];
                out.push_str(routed_name(word).unwrap_or(word));
            }
            _ => {
                let c = code[i..].chars().next().expect("a character starts here");
                i += c.len_utf8();
                out.push(c);
            }
        }
    }
    out
}

/// Device-provided replacements for runtime functions that cannot be lifted verbatim (they use a
/// device-absent libc). Each override is a self-contained `__device__` definition (its own helpers
/// inline) so reachability computed over the ORIGINAL unit stays valid. Keyed by defined symbol.
fn device_override(symbol: &str) -> Option<&'static str> {
    match symbol {
        "logos_fmt_f64" => Some(LOGOS_DEVICE_FLOAT_FMT),
        // Temporal clock reads: the C versions use `getenv`/`time` (device-absent). Return the globals the
        // host injected before launch (see CUDA_DEVICE_CLOCK / CUDA_HOST_TEMPLATE). Overriding the unit here
        // means its `getenv`/`time`/`atoll` never reach the device slice (the host_only_reason check below
        // is skipped for overridden units), so a temporal program lifts cleanly.
        "logos_today" => {
            Some("__device__ static int64_t logos_today(void) { return logos_dev_fixed_days; }\n")
        }
        "logos_now" => {
            Some("__device__ static int64_t logos_now(void) { return logos_dev_fixed_nanos; }\n")
        }
        // argv — the C runtime's `static _logos_argc/_logos_argv` are set from `main`'s argc/argv; the CUDA
        // host marshals them into these NON-static device globals (`cudaMemcpyToSymbol` needs external
        // linkage) before launch, and the lifted `logos_args()` reads them to build the `Seq of Text`.
        "_logos_argc" => Some("__device__ int _logos_argc = 0;\n"),
        "_logos_argv" => Some("__device__ char **_logos_argv = 0;\n"),
        _ => None,
    }
}

/// libc names that do NOT exist in CUDA device code and that no shim/override yet provides. A reachable
/// runtime unit mentioning one of these is refused (typed) — never miscompiled. As phases add device
/// shims (`calloc`/`realloc`/`strdup`/`snprintf`/…) their names leave this set. The value is the
/// user-facing reason.
fn host_only_reason(sym: &str) -> Option<&'static str> {
    match sym {
        "strtod" => Some("float parse (device strtod not yet provided)"),
        "vsprintf" => Some("string formatting (device vsprintf not yet provided)"),
        "atoll" | "atoi" | "atof" | "strtol" | "strtoll" => {
            Some("numeric parse (device parser not yet provided)")
        }
        "qsort" => Some("host sort (device qsort not yet provided)"),
        // The temporal shims (`logos_now`/`logos_today`) read the injected clock via `getenv`/`time`,
        // neither of which exists on device. A temporal program is already refused earlier at the VM-op
        // boundary (`LoadNow`/`LoadToday` are outside the CUDA fragment), so these shims never reach the
        // lift for a valid program; listing their libc here is the belt-and-suspenders that keeps a
        // FUTURE reachability path from silently miscompiling a device clock read.
        "getenv" | "time" => {
            Some("host clock (device getenv/time not provided — inject a constant instant)")
        }
        _ => None,
    }
}

/// `__device__`-annotate one runtime unit. Functions and file-scope globals get the qualifier (a
/// comment between `__device__` and the declarator is inert); typedefs are device-agnostic; `#…`
/// preprocessor lines are dropped (the CUDA preamble supplies the includes).
fn device_annotate(u: &CUnit) -> String {
    match u.kind {
        UnitKind::Typedef => u.text.trim_start().to_string(),
        UnitKind::Preproc => String::new(),
        UnitKind::Function | UnitKind::Global => format!("__device__ {}", u.text.trim_start()),
    }
}

/// Lift the whole program onto the device. Returns the self-contained `.cu`, or a typed refusal reason
/// if a reachable runtime unit needs a device facility we do not yet provide.
///
/// A concurrent program's C is the stepped tier's (`compile_to_c_stepped`): its runtime region carries the
/// stepped scheduler whole. The scheduler goes onto the device as the same sources under
/// [`CUDA_DEVICE_STEPPED_PROLOGUE`], at the place the host C carries it — after the runtime before it and
/// ahead of the runtime after it — with its preprocessor conditionals intact, which a tree-shake over units
/// could not keep; and every name it reaches in the C runtime is a root of the tree-shake.
pub fn lift_program_to_device(c_source: &str) -> Result<String, &'static str> {
    // The host collector reads process segments and stacks. Device allocations
    // use the allocator shims through the runtime's fallback entry points.
    let c_source = if let Some((before, rest)) = c_source.split_once("/* LC_GC_COLLECTOR_BEGIN */")
    {
        let (_, after) = rest
            .split_once("/* LC_GC_COLLECTOR_END */")
            .ok_or("unexpected C shape (unterminated host collector)")?;
        std::borrow::Cow::Owned(format!("{before}{after}"))
    } else {
        std::borrow::Cow::Borrowed(c_source)
    };
    let (runtime_src, user_src) = c_source
        .split_once(USER_CODE_SENTINEL)
        .ok_or("unexpected C shape (missing user-code sentinel)")?;
    let (before_src, after_src) = split_at_scheduler(runtime_src)?;
    let stepped = after_src.is_some();

    // Roots: every identifier the user region references, minus the two host-only argv globals it sets
    // (those setter lines are dropped in the lift, so the globals are never actually read on device), and
    // every identifier the scheduler references.
    let mut roots: BTreeSet<String> = c_identifiers(user_src);
    roots.remove("_logos_argc");
    roots.remove("_logos_argv");
    if stepped {
        roots.extend(c_identifiers(STEPPED_SCHEDULER));
    }

    let mut units = parse_c_units(before_src);
    let scheduler_at = units.len();
    units.extend(parse_c_units(after_src.unwrap_or("")));
    // Every macro goes onto the device whole (below), and one is expanded wherever it is named, so what a named
    // macro mentions is reached through it: a runtime function only a macro calls is kept.
    let macros: Vec<(String, BTreeSet<String>)> = units.iter().filter_map(macro_references).collect();
    let keep = loop {
        let keep = reachable_units(&units, &roots);
        let named: BTreeSet<&String> = keep
            .iter()
            .flat_map(|&i| units[i].references.iter())
            .chain(roots.iter())
            .collect();
        let through_macros: BTreeSet<String> = macros
            .iter()
            .filter(|(name, _)| named.contains(name))
            .flat_map(|(_, mentions)| mentions.iter())
            .filter(|m| !roots.contains(*m))
            .cloned()
            .collect();
        if through_macros.is_empty() {
            break keep;
        }
        roots.extend(through_macros);
    };

    let mut slice = String::new();
    // Emit every `#define` macro up-front: reachable code may reference an object-like constant
    // (e.g. `LBIG_BASE`) or function-like macro, and macros carry no dependency edges the reachability
    // pass can see. `#include` lines are dropped — the preamble supplies the headers.
    for u in &units {
        // A preproc unit may carry a leading doc comment, so check any line — not just the first.
        if u.kind == UnitKind::Preproc
            && u.text
                .lines()
                .any(|l| l.trim_start().starts_with("#define"))
        {
            slice.push_str(&route_alloc_to_shims(u.text.trim_start()));
            slice.push('\n');
        }
    }
    let mut placed_scheduler = !stepped;
    for &idx in &keep {
        if !placed_scheduler && idx >= scheduler_at {
            slice.push_str(&route_alloc_to_shims(&format!(
                "{CUDA_DEVICE_STEPPED_PROLOGUE}{STEPPED_SCHEDULER}"
            )));
            slice.push('\n');
            placed_scheduler = true;
        }
        let u = &units[idx];
        if u.defined.iter().any(|d| coroutine_only_names().contains(d)) {
            return Err("a coroutine-tier concurrency runtime (a GPU thread runs a concurrent program through the stepped tier)");
        }
        if let Some(sym) = u.defined.first() {
            if let Some(over) = device_override(sym) {
                slice.push_str(&route_alloc_to_shims(over));
                slice.push('\n');
                continue;
            }
        }
        // Cleanliness: refuse if this reachable unit names a device-absent libc we do not shim.
        if let Some(reason) = u.references.iter().find_map(|r| host_only_reason(r)) {
            return Err(reason);
        }
        slice.push_str(&route_alloc_to_shims(&device_annotate(u)));
        slice.push('\n');
    }
    if !placed_scheduler {
        slice.push_str(&route_alloc_to_shims(&format!(
            "{CUDA_DEVICE_STEPPED_PROLOGUE}{STEPPED_SCHEDULER}"
        )));
        slice.push('\n');
    }

    let user = route_alloc_to_shims(&lift_user_region(user_src));

    // argv: if the program calls `args()` (→ `logos_args`, which reads the device `_logos_argc`/`_logos_argv`
    // globals the `device_override` supplies), the HOST marshals its own argv into device memory and copies
    // it into those globals before the launch. Injected at the `/*LOGOS_ARGV_MARSHAL*/` marker; empty (the
    // marker vanishes) for a non-argv program, so its `.cu` is byte-identical to a world without argv.
    let argv_marshal = if roots.contains("logos_args") {
        "    { char **__dv = 0; cudaMalloc((void **)&__dv, (size_t)argc * sizeof(char *)); \
         char **__hp = (char **)malloc((size_t)argc * sizeof(char *)); \
         for (int __i = 0; __i < argc; __i++) { size_t __l = strlen(argv[__i]) + 1; char *__ds = 0; \
         cudaMalloc((void **)&__ds, __l); cudaMemcpy(__ds, argv[__i], __l, cudaMemcpyHostToDevice); __hp[__i] = __ds; } \
         cudaMemcpy(__dv, __hp, (size_t)argc * sizeof(char *), cudaMemcpyHostToDevice); \
         cudaMemcpyToSymbol(_logos_argc, &argc, sizeof(argc)); \
         cudaMemcpyToSymbol(_logos_argv, &__dv, sizeof(__dv)); free(__hp); }\n"
    } else {
        ""
    };
    // A concurrent run's host side: the seed read from `LOGOS_SEED` into the device before the launch, and a
    // failed run reported after it. Each marker line is dropped for a program with no tasks.
    let host_part = |text: &'static str| if stepped { text } else { "" };
    let host = CUDA_HOST_TEMPLATE
        .replace("/*LOGOS_ARGV_MARSHAL*/", argv_marshal)
        .replace(
            "/*LOGOS_HOST_SEED_READER*/\n",
            host_part(CUDA_HOST_SEED_READER),
        )
        .replace("/*LOGOS_HOST_RUN_SEED*/\n", host_part(CUDA_HOST_RUN_SEED))
        .replace(
            "/*LOGOS_HOST_RUN_FAILED*/\n",
            host_part(CUDA_HOST_RUN_FAILED),
        );

    Ok(format!(
        "{preamble}{alloc}{intrin}{rounding}{fault}{clock}{string}{slice}{user}{host}",
        preamble = CUDA_DEVICE_PREAMBLE,
        alloc = CUDA_DEVICE_ALLOC_SHIM,
        intrin = CUDA_DEVICE_INTRIN_SHIM,
        rounding = CUDA_DEVICE_INTEGRAL_ROUNDING,
        fault = CUDA_DEVICE_FAULT_SHIM,
        clock = CUDA_DEVICE_CLOCK,
        string = CUDA_DEVICE_STRING_SHIM,
    ))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_cuda/device_lift/tests.rs"]
mod tests;
