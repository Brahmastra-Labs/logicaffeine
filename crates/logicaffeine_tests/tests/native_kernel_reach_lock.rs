//! A backend that cannot link the standard library's native kernels refuses the kernels a program
//! can RUN — and only those.
//!
//! The loader splices whole standard-library modules into a program: a record named `HttpError`
//! demands the HTTP module, whose chain reaches `crypto.lg`, whose `## To native` functions compile
//! to `CallBuiltin(NativeKernel)`. None of that code runs unless something calls it. The gate and the
//! backend answer "what runs?" with one analysis (`vm::reach`), so a page that never calls a kernel
//! compiles, a program that does is refused naming the kernel, and a kernel the HOST can reach — a
//! page's failure handler, which only the host enters — counts as reachable.

use logicaffeine_compile::compile;

/// Every backend `refuse_unlinkable` gates, as `(name, outcome)`.
fn every_gated_backend(src: &str) -> Vec<(&'static str, Result<(), String>)> {
    vec![
        (
            "C (from bytecode)",
            compile::compile_to_c_via_bytecode(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "RISC-V",
            compile::compile_to_riscv(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "WASM",
            compile::compile_to_wasm(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
        (
            "soft core",
            logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words(src)
                .map(|_| ())
                .map_err(|e| format!("{e:?}")),
        ),
    ]
}

/// The shape of every typed-fetch page: a record the HTTP module is spliced in for.
const HTTP_ERROR_PAGE: &str = "\
## An HttpError has:
    a status, which is Int.
    a message, which is Text.

## Main
Let e be a new HttpError with status 404 and message \"x\".
Show e's status.
";

#[test]
fn a_page_that_never_calls_a_native_kernel_compiles_to_wasm() {
    let module = compile::compile_to_wasm(HTTP_ERROR_PAGE);
    assert!(
        module.is_ok(),
        "a program that never calls a native kernel was refused — the gate must judge the code that \
         can run, not every function the loader spliced in: {:?}",
        module.err()
    );
}

/// The backends that lower only the functions that can run. RISC-V and the soft core lower every
/// function, so an unused function's kernel call still stops them — in their own lowering.
const LOWERS_ONLY_WHAT_RUNS: &[&str] = &["C (from bytecode)", "WASM"];

#[test]
fn a_kernel_only_an_unused_function_calls_blocks_no_backend_that_lowers_what_runs() {
    let src = "\
## To unused (n: Int) -> Int:
    Let z be zerosW16(n).
    Return length of z.

## Main
Show 7.
";
    let refused: Vec<String> = every_gated_backend(src)
        .into_iter()
        .filter(|(backend, _)| LOWERS_ONLY_WHAT_RUNS.contains(backend))
        .filter_map(|(backend, r)| r.err().map(|e| format!("{backend}: {e}")))
        .collect();
    assert!(
        refused.is_empty(),
        "a kernel call in a function nothing calls cannot run, so it must not refuse the program:\n{}",
        refused.join("\n")
    );
}

#[test]
fn a_program_that_calls_a_native_kernel_is_still_refused_by_name() {
    let src = "\
## To shaped (n: Int) -> Int:
    Let z be zerosW16(n).
    Return length of z.

## Main
Show shaped(3).
";
    let wrong: Vec<String> = every_gated_backend(src)
        .into_iter()
        .filter_map(|(backend, r)| match r {
            Err(e) if e.contains("zerosW16") => None,
            Err(e) => Some(format!("{backend}: refused without naming the kernel: {e}")),
            Ok(()) => Some(format!(
                "{backend}: lowered a native-kernel call it cannot link"
            )),
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_kernel_the_failure_handler_calls_is_refused_though_no_code_calls_the_handler() {
    // The module never calls its failure handler — the HOST does, after a caught trap. So the
    // handler is a root of the call graph, and a kernel it reaches is code that can run.
    let src = "\
## Component App
State:
    Let why be 0.
Actions:
    When an action fails (e: Text):
        Let z be zerosW16(2).
        Set why to length of z.
    To boom:
        Let bad be item 9 of [1, 2].
View:
    <p>{why}</p><button id=\"b\" on-click={boom}>b</button>

## Main
Mount App.
";
    let r = compile::compile_to_wasm(src);
    match r {
        Err(e) => {
            let e = format!("{e:?}");
            assert!(
                e.contains("zerosW16"),
                "refused, but without naming the kernel: {e}"
            );
        }
        Ok(_) => panic!(
            "a kernel reachable from the page's failure handler was lowered — the host enters that \
             handler, so its code can run"
        ),
    }
}
