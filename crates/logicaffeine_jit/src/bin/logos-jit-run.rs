//! Execute an optimized LGB1 image on the VM plus copy-and-patch JIT.

#[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
fn main() {
    native::main();
}

#[cfg(not(all(target_arch = "x86_64", not(target_os = "windows"))))]
fn main() {
    eprintln!("logos-jit-run requires x86-64 on a non-Windows platform");
    std::process::exit(2);
}

#[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
mod native {
    use std::path::Path;

    use logicaffeine_compile::vm::NativeTier;

    pub(super) fn main() {
        logicaffeine_jit::segv_trace_install();
        let args: Vec<String> = std::env::args().collect();
        let Some(image_path) = args.get(1) else {
            eprintln!("usage: logos-jit-run IMAGE [ARGS...]");
            std::process::exit(2);
        };
        let bytes = match std::fs::read(Path::new(image_path)) {
            Ok(bytes) => bytes,
            Err(error) => refuse(error.to_string()),
        };
        let mut program_args = Vec::with_capacity(args.len());
        program_args.push("logos-jit-image".to_string());
        program_args.extend_from_slice(&args[2..]);
        let tier = logicaffeine_jit::ForgeTier::new();
        let outcome = match logicaffeine_jit::image::run(&bytes, &program_args, &tier) {
            Ok(outcome) => outcome,
            Err(error) => refuse(error),
        };
        trace(&tier);
        for line in outcome.lines {
            println!("{line}");
        }
        if let Some(error) = outcome.error {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }

    fn tracing() -> bool {
        std::env::var_os("LOGOS_ENGINE_TRACE").is_some()
    }

    fn refuse(error: String) -> ! {
        if tracing() {
            eprintln!("logos-engine: refused");
        }
        eprintln!("{error}");
        std::process::exit(1)
    }

    fn trace(tier: &logicaffeine_jit::ForgeTier) {
        if !tracing() {
            return;
        }
        eprintln!("logos-engine: vm+jit");
        eprintln!(
            "logos-engine: native-tier {}: {}",
            tier.engine_name(),
            tier.compile_summary()
        );
        let declines = tier.decline_summary();
        if !declines.is_empty() {
            eprintln!(
                "logos-engine: native-declines {}: {declines}",
                tier.engine_name()
            );
        }
        let opts = tier.optimization_summary();
        if !opts.is_empty() {
            eprintln!("logos-engine: native-opts {}: {opts}", tier.engine_name());
        }
        eprintln!(
            "logos-engine: native-runtime {}: {}",
            tier.engine_name(),
            tier.execution_summary()
        );
    }
}
