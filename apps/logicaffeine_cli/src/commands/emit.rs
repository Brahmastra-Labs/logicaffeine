//! `largo emit` — the unified code-emission verb, plus the direct-emit
//! backends shared by `largo build --emit` and `largo run --emit`.
//!
//! Every `EmitTarget`, by what it produces:
//!
//! - `rust`, `c` and `riscv` print (or `-o`-write) the generated Rust program,
//!   the C translation, or RV64GC assembly; `riscv -o` also writes the
//!   `logos_rt.c` runtime shim beside the assembly.
//! - `wasm` and `wasm-linked` compile straight to a `.wasm` module via the
//!   built-in backend (no rustc, cargo, or wasm-bindgen) with a Node.js host
//!   shim beside it; `wasm-linked` also links the real Rust runtime (big
//!   integers, exact numbers, the exact extension's values) with `rust-lld`. `wasm-dom` adds a browser host (`.html` + `.js`) that wires
//!   the module's `dom_*` imports to the real DOM.
//! - `tenstorrent`, `cuda`, `analog` and `plasticine` write a bundle with
//!   `-o DIR` (a single-core TT-Metalium program, a single-file CUDA program,
//!   a mixed-signal analog bundle, a Spatial bundle for Plasticine) and
//!   print its central artifact when bare.
//! - `rdu` targets the SambaNova RDU: a certified neutral placement, with
//!   `--chip` choosing the chip, `--format` the artifact, and `--bridge` an
//!   optional vendor SDK.

use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::require_project_root;
use crate::project::manifest::Manifest;
use crate::ui;

/// The code targets `largo emit` can produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum EmitTarget {
    /// The generated Rust program (what `largo build` hands to cargo).
    Rust,
    /// The C translation: one self-contained C file with its runtime embedded.
    C,
    /// RV64GC assembly (the RISC-V AOT tier) — assemble+link with a RISC-V GCC, run under qemu.
    Riscv,
    /// A self-contained `.wasm` module + Node.js host shim (no toolchain).
    Wasm,
    /// A `.wasm` module linked against the real Rust runtime (rust-lld).
    WasmLinked,
    /// A self-contained `.wasm` DOM app + a browser host (`.html` + `.js`) that
    /// wires the `dom_*` imports to the real DOM and events to the LOGOS trampoline.
    WasmDom,
    /// A single-Tensix-core TT-Metalium program (the LOGOS → Tenstorrent AOT tier):
    /// a data-movement compute kernel for a baby RISC-V core + a host orchestrator +
    /// CMake glue + the ≡VM C oracle. `-o DIR` writes the bundle; bare prints the kernel.
    Tenstorrent,
    /// A single-file CUDA program (the LOGOS → CUDA AOT tier): a `__global__` kernel
    /// running a whole LOGOS program on one GPU thread + host launcher + Makefile + the
    /// ≡VM C oracle. `-o DIR` writes the bundle; bare prints the `.cu`.
    Cuda,
    /// A mixed-signal analog bundle (the LOGOS → analog spatial fabric tier): the region's
    /// tile netlist + gain schedule + range certificate + itemized error budget + placement +
    /// a behavioral ngspice netlist, for the modeled sky130-class FPAA. `-o DIR` writes the
    /// bundle; bare prints the SPICE netlist.
    Analog,
    /// A Plasticine bundle (the LOGOS → open-RDU lane, Prabhakar et al. ISCA 2017) for the first function,
    /// a counted loop: Spatial source (`Logos_<fn>.scala`), the same loop as a Spatial regression-harness app
    /// checking the VM's answer (`Logos_<fn>_test.scala`, what the public toolchain's gate runs), the
    /// checkerboard placement + metapipeline II model (`placement.json`) and the MODELED report row
    /// (`report.json`). `-o DIR` writes the bundle; bare prints the Spatial source.
    Plasticine,
    /// The SambaNova RDU — a certified neutral placement, SDK-gated for the vendor config. ONE front door,
    /// the lane chosen by program shape and printed by name (tensor / loop / whole-program), the chip's
    /// descriptor by `--chip sn40l|sn50`, the artifact by `--format`, the vendor SDK by `--bridge`. `-o DIR`
    /// writes `neutral.json` + `placement.txt` + `certificate.json` + `report.json` (+ the format's files);
    /// bare prints the placement text (or the chosen format) with the lane and chip on stderr.
    Rdu,
}

/// What `largo emit rdu` prints bare / adds to its bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum RduFormat {
    /// The neutral placement text (one line per IR object).
    Placement,
    /// Spatial source (`@spatial class Logos_<name> extends SpatialTest`) — `spatial.scala`.
    Spatial,
    /// A PyTorch module + weights manifest + SambaFlow compile scaffold — `model.py`, `compile.py`, `manifest.json`.
    Sambaflow,
    /// The Qwen3.8 deployment plan for the chip's vLLM backend — `vllm_plan.json`.
    Vllm,
    /// The MODELED report: lane, certificate, census, the II view — `report.json`.
    Report,
}

/// The RDU dials, refused by name on any other target.
fn refuse_rdu_dials(
    target: EmitTarget,
    chip: &Option<String>,
    format: &Option<RduFormat>,
    bridge: &Option<String>,
    cache: &Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let name = clap::ValueEnum::to_possible_value(&target)
        .map(|v| v.get_name().to_string())
        .unwrap_or_default();
    for (flag, given) in [
        ("--chip", chip.is_some()),
        ("--format", format.is_some()),
        ("--bridge", bridge.is_some()),
        ("--cache", cache.is_some()),
    ] {
        if given {
            return Err(ui::CliError::with_hint(
                format!("`{flag}` applies to `emit rdu`, not `emit {name}`"),
                "drop the flag, or emit the rdu target",
            )
            .exit_code(ui::EXIT_USAGE)
            .into());
        }
    }
    Ok(())
}

/// Handle `largo emit <target> [FILE] [-o PATH] [--chip C] [--format F] [--bridge B]`.
pub(crate) fn cmd_emit(
    target: EmitTarget,
    file: Option<PathBuf>,
    output: Option<PathBuf>,
    chip: Option<String>,
    format: Option<RduFormat>,
    bridge: Option<String>,
    model: Option<String>,
    ctx: Option<usize>,
    batch: Option<String>,
    tp: Option<String>,
    prefill: Option<usize>,
    cache: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    if ctx.is_some() && model.is_none() {
        return Err(ui::CliError::with_hint(
            "`--ctx` sizes the KV cache of a `--model` decode bundle".to_string(),
            "add --model, or drop --ctx",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    if batch.is_some() && ctx.is_none() {
        return Err(ui::CliError::with_hint(
            "`--batch` sizes a `--model --ctx` decode bundle".to_string(),
            "add --model and --ctx, or drop --batch",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    if tp.is_some() && ctx.is_none() {
        return Err(ui::CliError::with_hint(
            "`--tp` spreads a `--model --ctx` decode bundle over chips".to_string(),
            "add --model and --ctx, or drop --tp",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    if prefill.is_some() && ctx.is_none() {
        return Err(ui::CliError::with_hint(
            "`--prefill` fills the KV cache of a `--model --ctx` bundle".to_string(),
            "add --model and --ctx, or drop --prefill",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    if target != EmitTarget::Rdu {
        refuse_rdu_dials(target, &chip, &format, &bridge, &cache)?;
        if model.is_some() {
            return Err(ui::CliError::with_hint(
                "`--model` applies to `emit rdu`".to_string(),
                "drop the flag, or emit the rdu target",
            )
            .exit_code(ui::EXIT_USAGE)
            .into());
        }
    }
    if let (EmitTarget::Rdu, Some(m)) = (target, model.as_deref()) {
        return emit_rdu_model(m, ctx, batch, tp, prefill, file, output, chip, cache.as_deref());
    }
    match target {
        EmitTarget::Rdu => {
            use logicaffeine_compile::spatial::rdu_chip::RduChip;
            let chip = match chip.as_deref() {
                Some(s) => RduChip::parse(s).ok_or_else(|| {
                    ui::CliError::with_hint(
                        format!("unknown RDU chip `{s}`"),
                        "expected one of: sn40l, sn50",
                    )
                    .exit_code(ui::EXIT_USAGE)
                })?,
                None => RduChip::Sn50,
            };
            let entry = resolve_entry(file)?;
            let source = assembled_source(&entry)?;
            // The lane is a function of the program's shape, known before anything is lowered — printed
            // first so a refusal still says which lane refused.
            let lane = logicaffeine_compile::spatial::lower::rdu_lanes::lane_of(&source);
            ui::note(format!(
                "rdu lane: {} chip: {} target: {}",
                lane.as_str(),
                chip.as_str(),
                chip.target_name()
            ));
            // ★ A VENDOR-INGESTION ARTIFACT DOES NOT COME FROM THE LOWERING, SO IT DOES NOT WAIT FOR IT.
            //
            // `--format sambaflow` reads the tensor sentences and writes a PyTorch module; `--format vllm`
            // is derived from the chip facts and the model config. Neither reads the spatial IR. Computing
            // the certified bundle first made both of them COST the lowering anyway — measured on the real
            // 64-layer Qwen3.8: the torch module is 446 ms and 264 KB, the lowering is tens of minutes and
            // hundreds of gigabytes, and on the full model it does not finish at all. So the day-one
            // artifact was gated behind the one step that cannot currently run at day-one scale.
            //
            // They are now produced and written FIRST. The certified bundle still follows, because a
            // vendor artifact is better with a certificate beside it — but it is no longer a precondition
            // for getting the thing you asked for.
            let fmt = format.unwrap_or(RduFormat::Placement);
            if bridge.is_none() {
                if let (Some(dir), Some(extra)) =
                    (output.as_deref(), vendor_extra(&source, chip, fmt)?)
                {
                    write_vendor(dir, &extra)?;
                    ui::note(format!(
                        "wrote the {} artifact to {} before lowering — it does not derive from the spatial IR; the certified bundle follows",
                        match extra { Extra::Sambaflow(_) => "SambaFlow", _ => "vLLM" },
                        dir.display()
                    ));
                }
            }
            let art = through_cache(cache.as_deref(), || logicaffeine_compile::compile::compile_to_rdu(&source, chip))?
                .map_err(rdu_refusal)?;
            emit_rdu_artifact(
                &source,
                art,
                chip,
                fmt,
                bridge.as_deref(),
                output.as_deref(),
            )
        }
        EmitTarget::Rust => {
            let entry = resolve_entry(file)?;
            crate::diagnostics::front_end(&entry, &dep_roots_for_entry(&entry))?;
            let code = crate::compile::compile_project(&entry)?.rust_code;
            write_emitted(&code, output.as_deref())
        }
        EmitTarget::C => {
            let entry = resolve_entry(file)?;
            let closure = assembled(&entry)?;
            let code = logicaffeine_compile::compile::compile_to_c(&closure.source)
                .map_err(|e| failed("C emission", &e, &closure, &entry))?;
            write_emitted(&code, output.as_deref())?;
            write_value_runtime(&code, output.as_deref())
        }
        EmitTarget::Riscv => {
            let entry = resolve_entry(file)?;
            let closure = assembled(&entry)?;
            let asm = logicaffeine_compile::compile::compile_to_riscv(&closure.source)
                .map_err(|e| failed("RISC-V emission", &e, &closure, &entry))?;
            write_emitted(&asm, output.as_deref())?;
            // What the assembly links is written beside the `-o` output (stdout mode prints only the assembly).
            match output.as_deref() {
                Some(path) => {
                    let line = write_riscv_runtime(&asm, path)?;
                    ui::info(format!(
                        "Link with: riscv64-linux-gnu-gcc -O2 -static -o {} {} {}",
                        path.with_extension("").display(),
                        path.display(),
                        line.join(" ")
                    ));
                }
                None if logicaffeine_compile::codegen_riscv::links_value_runtime(&asm) => ui::note(
                    "This program links the exact extension's value runtime: emit with -o to write it beside the assembly",
                ),
                None => {}
            }
            Ok(())
        }
        EmitTarget::Wasm => emit_wasm_target(file, output, false, false),
        EmitTarget::WasmLinked => emit_wasm_target(file, output, true, false),
        EmitTarget::WasmDom => emit_wasm_target(file, output, false, true),
        EmitTarget::Tenstorrent => {
            let entry = resolve_entry(file)?;
            let closure = assembled(&entry)?;
            let art = logicaffeine_compile::compile::compile_to_tenstorrent(&closure.source)
                .map_err(|e| failed("Tenstorrent emission", &e, &closure, &entry))?;
            match output.as_deref() {
                // `-o DIR`: write the whole single-core TT-Metalium bundle.
                Some(dir) => {
                    fs::create_dir_all(dir)?;
                    let kernel = dir.join(logicaffeine_compile::codegen_tt::KERNEL_FILE_NAME);
                    fs::write(&kernel, &art.kernel_cpp)?;
                    fs::write(dir.join("logos_tt_host.cpp"), &art.host_cpp)?;
                    fs::write(dir.join("CMakeLists.txt"), &art.cmake)?;
                    fs::write(dir.join("logos_oracle.c"), &art.oracle_c)?;
                    ui::info(format!("Wrote TT-Metalium bundle to {}", dir.display()));
                    ui::info("Build (needs a tt-metal SDK): TT_METAL_HOME=… cmake -B build && cmake --build build".to_string());
                    Ok(())
                }
                // Bare: print the compute kernel (the interesting artifact) to stdout.
                None => {
                    println!("{}", art.kernel_cpp);
                    Ok(())
                }
            }
        }
        EmitTarget::Analog => {
            let entry = resolve_entry(file)?;
            let closure = assembled(&entry)?;
            let art = logicaffeine_compile::compile::compile_to_analog(&closure.source)
                .map_err(|e| failed("analog emission", &e, &closure, &entry))?;
            match output.as_deref() {
                Some(dir) => {
                    fs::create_dir_all(dir)?;
                    fs::write(dir.join("netlist.sp"), &art.spice)?;
                    fs::write(
                        dir.join("schedule.json"),
                        format!(
                            "{{\n  \"region\": \"{}\",\n  \"scale_log2\": {},\n  \"out_descale_log2\": {},\n  \"mul_vref_v\": {},\n  \"tiles\": {}\n}}\n",
                            art.fn_name,
                            art.schedule.scale_log2,
                            art.schedule.out_descale_log2,
                            art.schedule.mul_vref_v,
                            art.netlist.cells.len()
                        ),
                    )?;
                    let terms: Vec<String> = art
                        .budget
                        .terms
                        .iter()
                        .map(|t| {
                            format!(
                                "    {{ \"eps_math\": {:e}, \"source\": \"{:?}\" }}",
                                t.eps_math, t.source
                            )
                        })
                        .collect();
                    fs::write(
                        dir.join("budget.json"),
                        format!(
                            "{{\n  \"eps_math\": {:e},\n  \"terms\": [\n{}\n  ]\n}}\n",
                            art.budget.eps_math,
                            terms.join(",\n")
                        ),
                    )?;
                    let nets: Vec<String> = art
                        .cert
                        .per_net
                        .iter()
                        .enumerate()
                        .map(|(i, n)| {
                            format!(
                                "    {{ \"net\": {i}, \"volt_lo\": {}, \"volt_hi\": {} }}",
                                n.volt_lo, n.volt_hi
                            )
                        })
                        .collect();
                    fs::write(
                        dir.join("cert.json"),
                        format!(
                            "{{\n  \"rail_half_span_v\": {},\n  \"nets\": [\n{}\n  ]\n}}\n",
                            art.cert.rail_half_span_v,
                            nets.join(",\n")
                        ),
                    )?;
                    ui::info(format!("Wrote analog bundle to {}", dir.display()));
                    ui::info("Simulate (needs ngspice): ngspice -b netlist.sp".to_string());
                    Ok(())
                }
                None => {
                    println!("{}", art.spice);
                    Ok(())
                }
            }
        }
        EmitTarget::Plasticine => {
            let entry = resolve_entry(file)?;
            let closure = assembled(&entry)?;
            let art = logicaffeine_compile::compile::compile_to_plasticine(&closure.source)
                .map_err(|e| failed("Plasticine emission", &e, &closure, &entry))?;
            match output.as_deref() {
                // `-o DIR`: Spatial source + placement + the report row.
                Some(dir) => {
                    fs::create_dir_all(dir)?;
                    fs::write(
                        dir.join(format!("Logos_{}.scala", art.fn_name)),
                        &art.spatial,
                    )?;
                    fs::write(
                        dir.join(format!("Logos_{}_test.scala", art.fn_name)),
                        &art.spatial_test,
                    )?;
                    fs::write(dir.join("placement.json"), art.placement_json())?;
                    fs::write(dir.join("report.json"), art.row_json())?;
                    ui::info(format!(
                        "Wrote Plasticine bundle to {} (target {}, MODELED)",
                        dir.display(),
                        art.target
                    ));
                    ui::info("Simulate (needs the open toolchain): Spatial → PIR → plastiroute → Tungsten, see scripts/plasticine/README.md".to_string());
                    Ok(())
                }
                // Bare: print the Spatial source (the interesting artifact) to stdout.
                None => {
                    println!("{}", art.spatial);
                    Ok(())
                }
            }
        }
        EmitTarget::Cuda => {
            let entry = resolve_entry(file)?;
            let closure = assembled(&entry)?;
            let art = logicaffeine_compile::compile::compile_to_cuda(&closure.source)
                .map_err(|e| failed("CUDA emission", &e, &closure, &entry))?;
            match output.as_deref() {
                // `-o DIR`: write the single-file CUDA program + Makefile + ≡VM C oracle.
                Some(dir) => {
                    fs::create_dir_all(dir)?;
                    let kernel = dir.join(logicaffeine_compile::codegen_cuda::KERNEL_FILE_NAME);
                    fs::write(&kernel, &art.kernel_cu)?;
                    fs::write(dir.join("Makefile"), &art.makefile)?;
                    fs::write(dir.join("logos_oracle.c"), &art.oracle_c)?;
                    ui::info(format!("Wrote CUDA bundle to {}", dir.display()));
                    ui::info(
                        "Build (needs the CUDA toolkit): make  # nvcc -arch=sm_86".to_string(),
                    );
                    Ok(())
                }
                // Bare: print the `.cu` (the interesting artifact) to stdout.
                None => {
                    println!("{}", art.kernel_cu);
                    Ok(())
                }
            }
        }
    }
}

/// `compile` through the section cache `--cache DIR` names ([`logicaffeine_compile::spatial::compile_cache`]), saying
/// what it compiled and reused.
fn through_cache<R>(dir: Option<&Path>, compile: impl FnOnce() -> R) -> Result<R, Box<dyn std::error::Error>> {
    let (made, said) = logicaffeine_compile::spatial::compile_cache::compile_through(dir, compile)
        .map_err(|e| ui::CliError::new(format!("the section cache: {e}")))?;
    if let Some(line) = said {
        ui::note(line);
    }
    Ok(made)
}

/// The RDU backend's typed refusal, as the one line it was written as.
fn rdu_refusal(e: logicaffeine_compile::error::ParseError) -> String {
    match e.kind {
        logicaffeine_compile::error::ParseErrorKind::Custom(msg) => msg,
        other => format!("rdu backend: {other:?}"),
    }
}

/// Write or print an RDU artifact: the four files always, the format's files beside them, and the bridge's
/// config when one was named. Bare mode prints ONE artifact to stdout (the bridge's config, else the
/// format's) and says which lane and chip on stderr, so a redirect captures exactly the artifact.
/// `largo emit rdu --model M -o DIR`: the whole model as one certified program per layer kind, the head,
/// and a manifest binding every layer — written program by program, so the command holds one at a time.
/// A decode dial's value: a count, or `auto` for the serving plan's choice.
enum Dial {
    Auto,
    Count(usize),
}

fn dial(flag: &str, v: Option<String>) -> Result<Option<Dial>, Box<dyn std::error::Error>> {
    match v.as_deref() {
        None => Ok(None),
        Some("auto") => Ok(Some(Dial::Auto)),
        Some(n) => n.parse().map(|n| Some(Dial::Count(n))).map_err(|_| {
            ui::CliError::with_hint(
                format!("`--{flag} {n}` is neither a count nor `auto`"),
                format!("pass --{flag} 8, or --{flag} auto"),
            )
            .exit_code(ui::EXIT_USAGE)
            .into()
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_rdu_model(
    model: &str,
    ctx: Option<usize>,
    batch: Option<String>,
    tp: Option<String>,
    prefill: Option<usize>,
    file: Option<PathBuf>,
    output: Option<PathBuf>,
    chip: Option<String>,
    cache: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
    use logicaffeine_compile::compile::RduModel;
    use logicaffeine_compile::spatial::rdu_chip::RduChip;
    use std::fs;
    if file.is_some() {
        return Err(ui::CliError::with_hint(
            "`--model` names the model; it does not take FILE".to_string(),
            "drop FILE, or drop --model",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    // A built-in configuration, or a checkpoint on disk — a GGUF file or a Hugging Face directory — read as its spec,
    // each weight the author declares Q8_0 read as stored where the checkpoint stores it so and densely otherwise.
    let builtin = match model {
        "qwen38" => Some(Qwen38Cfg::real()),
        "qwen38-tiny" => Some(Qwen38Cfg::tiny()),
        "qwen38-mini" => Some(Qwen38Cfg::mini()),
        _ => None,
    };
    let path = std::path::Path::new(model);
    if builtin.is_none() && !path.exists() {
        return Err(ui::CliError::with_hint(
            format!("unknown model `{model}`"),
            "expected one of: qwen38, qwen38-tiny, qwen38-mini, a GGUF checkpoint, or a Hugging Face checkpoint directory",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    let mut checkpoint = match builtin {
        Some(_) => None,
        None => Some(logicaffeine_compile::model::open_checkpoint(path).map_err(|e| ui::CliError::new(format!("{model}: {e}")))?),
    };
    let rdu_model = match (&builtin, checkpoint.as_deref_mut()) {
        (Some(cfg), _) => RduModel::qwen38(cfg),
        (None, Some(c)) => {
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| model.to_string());
            RduModel::from_checkpoint(&name, c).map_err(ui::CliError::new)?
        }
        (None, None) => unreachable!("a model is built in or opened"),
    };
    let spec = &rdu_model.spec;
    let Some(dir) = output else {
        return Err(ui::CliError::with_hint(
            "`--model` writes a bundle directory".to_string(),
            "pass -o DIR",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    };
    let chip = match chip.as_deref() {
        Some(s) => RduChip::parse(s).ok_or_else(|| {
            ui::CliError::with_hint(
                format!("unknown RDU chip `{s}`"),
                "expected one of: sn40l, sn50",
            )
            .exit_code(ui::EXIT_USAGE)
        })?,
        None => RduChip::Sn50,
    };
    if ctx == Some(0) {
        return Err(ui::CliError::with_hint(
            "`--ctx 0` is a KV cache with no positions".to_string(),
            "pass the context length, e.g. --ctx 4096",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    // The `auto` dials read the serving plan — the chips' memory hierarchy and scale-up domain against the model.
    let (batch, tp) = (dial("batch", batch)?, dial("tp", tp)?);
    let plan = || {
        Qwen38Cfg::from_spec(spec)
            .map_err(|_| format!("the serving plan sizes the Qwen3.5/3.8 hybrid, not {} (CHIP-5)", spec.arch))
            .and_then(|cfg| logicaffeine_compile::spatial::fit::qwen38_serving_plan(&cfg, &chip.facts(), ctx.unwrap_or(1)))
        .map_err(|e| {
            ui::CliError::with_hint(
                format!("the serving plan for {model} on the {}: {e}", chip.as_str()),
                "pass --tp and --batch as counts",
            )
            .exit_code(ui::EXIT_USAGE)
        })
    };
    let batch = match batch {
        Some(Dial::Auto) => Some(plan()?.sequences_resident_in_sram),
        Some(Dial::Count(n)) => Some(n),
        None => None,
    };
    let tp = match tp {
        Some(Dial::Auto) => Some(plan()?.tensor_parallel),
        Some(Dial::Count(n)) => Some(n),
        None => None,
    };
    if batch == Some(0) {
        return Err(ui::CliError::with_hint(
            "`--batch 0` serves no sequence".to_string(),
            "pass the sequences per step, e.g. --batch 8",
        )
        .exit_code(ui::EXIT_USAGE)
        .into());
    }
    match (prefill, ctx) {
        (Some(0), _) => {
            return Err(ui::CliError::with_hint(
                "`--prefill 0` is a chunk of no tokens".to_string(),
                "pass the prompt tokens per step, e.g. --prefill 512",
            )
            .exit_code(ui::EXIT_USAGE)
            .into())
        }
        (Some(n), Some(c)) if n > c => {
            return Err(ui::CliError::with_hint(
                format!("`--prefill {n}` is longer than the {c}-position KV cache"),
                "pass a chunk no longer than --ctx",
            )
            .exit_code(ui::EXIT_USAGE)
            .into())
        }
        (Some(n), _) if batch.is_some_and(|b| b > 1) || tp.is_some_and(|t| t > 1) => {
            return Err(ui::CliError::with_hint(
                format!("`--prefill {n}` is one sequence on one chip"),
                "drop --batch and --tp for a prefill bundle",
            )
            .exit_code(ui::EXIT_USAGE)
            .into())
        }
        _ => {}
    }
    if let Some(n) = tp {
        if let Err(r) = logicaffeine_compile::spatial::fit::admits_tp(spec, n) {
            let domain = logicaffeine_compile::spatial::fit::default_tensor_parallel(&chip.facts());
            let widths: Vec<String> = (1..=domain)
                .filter(|w| logicaffeine_compile::spatial::fit::admits_tp(spec, *w).is_ok())
                .map(|w| w.to_string())
                .collect();
            return Err(ui::CliError::with_hint(
                format!("`--tp {n}`: {}", r.why),
                format!("{model} splits over {} chip(s)", widths.join(", ")),
            )
            .exit_code(ui::EXIT_USAGE)
            .into());
        }
    }
    let decode = ctx.map(|ctx| logicaffeine_compile::compile::StepShape {
        ctx,
        batch: batch.unwrap_or(1),
        tp: tp.unwrap_or(1),
        prefill,
    });
    let form = match decode {
        Some(d) if d.prefill.is_some() => format!(
            "prefill, a chunk of {} tokens of one sequence, KV cache of {} positions",
            d.prefill.unwrap_or(0),
            d.ctx
        ),
        Some(d) => {
            let seqs = if d.batch > 1 {
                format!("{} sequences per step, ", d.batch)
            } else {
                String::new()
            };
            let chips = if d.tp > 1 {
                format!("tensor-parallel over {} chips, ", d.tp)
            } else {
                String::new()
            };
            format!(
                "decode, {seqs}{chips}KV cache of {} positions{}",
                d.ctx,
                if d.batch > 1 { " each" } else { "" }
            )
        }
        None => "the authored forward".to_string(),
    };
    ui::note(format!(
        "rdu model: {model} ({} layers, d={}) chip: {} target: {} form: {form}",
        spec.layers.len(),
        spec.d,
        chip.as_str(),
        chip.target_name()
    ));
    fs::create_dir_all(&dir)?;
    let bundle = through_cache(cache, || logicaffeine_compile::compile::compile_model_to_rdu_streamed(
        &rdu_model,
        chip,
        "weights",
        decode,
        |f| {
            let sub = dir.join(&f.dir);
            fs::create_dir_all(&sub)?;
            for (name, text) in f.files() {
                fs::write(sub.join(name), text)?;
            }
            ui::note(format!(
                "wrote {}/ ({})",
                f.dir,
                f.files().map(|(name, _)| name).join(", ")
            ));
            Ok(())
        },
    ))??;
    for (path, bytes) in &bundle.constants {
        let file = dir.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&file, bytes)?;
        ui::note(format!(
            "wrote {path} (a model constant the manifest binds)"
        ));
    }
    fs::write(dir.join("manifest.json"), &bundle.manifest)?;
    fs::write(dir.join("program.lg"), &bundle.program)?;
    ui::note("wrote program.lg (the model's program the bundle deploys, as the manifest's `program` line names it)");
    ui::info(format!(
        "Wrote the {model} RDU bundle to {} (MODELED, SDK-gated)",
        dir.display()
    ));
    Ok(())
}

/// What a `--format` adds beside the certified bundle.
enum Extra {
    None,
    Spatial(String),
    Sambaflow(logicaffeine_compile::spatial::lower::sambaflow::TorchArtifact),
    Vllm(String),
}

/// The VENDOR-INGESTION artifact for a format, when that format has one. `None` for the formats whose
/// content comes out of the spatial lowering, which cannot be produced without it.
///
/// One definition, two callers: once before the lowering (so the artifact lands even when the bundle is
/// expensive or impossible) and once inside the emitter (so a single run still produces both).
fn vendor_extra(
    source: &str,
    chip: logicaffeine_compile::spatial::rdu_chip::RduChip,
    format: RduFormat,
) -> Result<Option<Extra>, Box<dyn std::error::Error>> {
    use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
    use logicaffeine_compile::spatial::fit::{default_tensor_parallel, render, vllm_plan};
    use logicaffeine_compile::spatial::lower::sambaflow::emit_torch_module;
    match format {
        RduFormat::Sambaflow => Ok(Some(Extra::Sambaflow(
            emit_torch_module(source)
                .map_err(|e| format!("rdu backend: --format sambaflow: {e}"))?,
        ))),
        RduFormat::Vllm => {
            let facts = chip.facts();
            Ok(Some(Extra::Vllm(render(
                &vllm_plan(&Qwen38Cfg::real(), &facts, default_tensor_parallel(&facts))
                    .map_err(|e| format!("rdu backend: --format vllm: {e}"))?,
            ))))
        }
        _ => Ok(None),
    }
}

/// Write a format's own files into the bundle directory.
fn write_vendor(dir: &Path, extra: &Extra) -> std::io::Result<Vec<&'static str>> {
    use std::fs;
    fs::create_dir_all(dir)?;
    Ok(match extra {
        Extra::None => Vec::new(),
        Extra::Spatial(s) => {
            fs::write(dir.join("spatial.scala"), s)?;
            vec!["spatial.scala"]
        }
        Extra::Sambaflow(t) => {
            fs::write(dir.join("model.py"), &t.model_py)?;
            fs::write(dir.join("compile.py"), &t.compile_py)?;
            fs::write(dir.join("manifest.json"), &t.manifest_json)?;
            vec!["model.py", "compile.py", "manifest.json"]
        }
        Extra::Vllm(p) => {
            fs::write(dir.join("vllm_plan.json"), p)?;
            vec!["vllm_plan.json"]
        }
    })
}

fn emit_rdu_artifact(
    source: &str,
    art: logicaffeine_compile::compile::RduArtifact,
    chip: logicaffeine_compile::spatial::rdu_chip::RduChip,
    format: RduFormat,
    bridge: Option<&str>,
    output: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    use logicaffeine_compile::spatial::lower::rdu::GATED_REASON;
    use logicaffeine_compile::spatial::lower::rdu_sdk::{bridge_by_name, BRIDGE_NAMES};

    let target_name = chip.target_name();
    let extra = match format {
        RduFormat::Placement | RduFormat::Report => Extra::None,
        RduFormat::Spatial => match &art.spatial_source {
            Some(s) => Extra::Spatial(s.clone()),
            None => {
                return Err(format!(
                    "rdu backend: --format spatial: the {} lane produced no Spatial source ({})",
                    art.lane.as_str(),
                    art.notes.join("; ")
                )
                .into())
            }
        },
        RduFormat::Sambaflow | RduFormat::Vllm => {
            vendor_extra(source, chip, format)?.expect("both vendor formats have an artifact")
        }
    };
    let bridged = match bridge {
        None => None,
        Some(name) => {
            let b = bridge_by_name(name).ok_or_else(|| {
                format!(
                    "rdu bridge: unknown bridge `{name}` — the bridges there are: {}",
                    BRIDGE_NAMES.join(", ")
                )
            })?;
            let neutral = art.neutral.as_ref().ok_or_else(|| format!("rdu bridge: the {} lane's artifact carries no spatial program for `{name}` to ingest", art.lane.as_str()))?;
            let config = b.ingest(neutral).map_err(|e| e.to_string())?;
            Some((b.name().to_string(), config))
        }
    };
    let pretty =
        |v: &serde_json::Value| format!("{}\n", serde_json::to_string_pretty(v).expect("JSON"));
    match output {
        Some(dir) => {
            fs::create_dir_all(dir)?;
            fs::write(dir.join("neutral.json"), &art.neutral_json)?;
            fs::write(dir.join("placement.txt"), &art.placement_txt)?;
            fs::write(dir.join("certificate.json"), &art.certificate_json)?;
            fs::write(dir.join("report.json"), &art.report_json)?;
            let mut wrote = vec![
                "neutral.json",
                "placement.txt",
                "certificate.json",
                "report.json",
            ];
            if let Some(whole) = &art.whole_certificate_json {
                fs::write(dir.join(logicaffeine_compile::spatial::lower::rdu::WHOLE_CERTIFICATE_FILE), whole)?;
                wrote.push(logicaffeine_compile::spatial::lower::rdu::WHOLE_CERTIFICATE_FILE);
            }
            wrote.extend(write_vendor(dir, &extra)?);
            if let Some((name, config)) = &bridged {
                let config_file = if config.format.ends_with("+json") {
                    "vendor_config.json"
                } else {
                    "vendor_config.bin"
                };
                fs::write(dir.join(config_file), &config.bytes)?;
                let meta = serde_json::json!({
                    "bridge": name,
                    "format": config.format,
                    "certificate_digest": config.certificate_digest,
                    "config_file": config_file,
                    "config_bytes": config.bytes.len(),
                });
                fs::write(dir.join("bridge.json"), pretty(&meta))?;
                wrote.extend([config_file, "bridge.json"]);
            }
            ui::info(format!(
                "Wrote RDU bundle to {} ({}) — lane: {}, chip: {}, target: {} (MODELED, SDK-gated)",
                dir.display(),
                wrote.join(", "),
                art.lane.as_str(),
                chip.as_str(),
                target_name
            ));
            ui::info(format!("Gated: {GATED_REASON}"));
        }
        None => {
            if let Some((name, config)) = &bridged {
                match std::str::from_utf8(&config.bytes) {
                    Ok(text) => print!("{text}"),
                    Err(_) => print!(
                        "{}",
                        pretty(
                            &serde_json::json!({ "bridge": name, "format": config.format, "certificate_digest": config.certificate_digest, "config_bytes": config.bytes.len() })
                        )
                    ),
                }
                ui::note(format!(
                    "rdu bridge: {name} → {} ({} bytes), certificate digest {}",
                    config.format,
                    config.bytes.len(),
                    config.certificate_digest
                ));
            } else {
                match &extra {
                    Extra::None if format == RduFormat::Report => print!("{}", art.report_json),
                    Extra::None => print!("{}", art.placement_txt),
                    Extra::Spatial(s) => print!("{s}"),
                    Extra::Sambaflow(t) => print!("{}", t.model_py),
                    Extra::Vllm(p) => print!("{p}"),
                }
            }
            ui::note(format!(
                "rdu lane: {} chip: {} target: {} — {GATED_REASON}",
                art.lane.as_str(),
                chip.as_str(),
                target_name
            ));
        }
    }
    for n in &art.notes {
        ui::note(format!("rdu note: {n}"));
    }
    Ok(())
}

/// Assemble the whole project reachable from `entry` (imports across files and
/// directories) into one source string — the same closure seam `largo
/// test`/`serve`/`build`/`run` consume, so every emit backend (C, wasm, isa)
/// sees imported functions and types AND the linked `path`/`git`-local deps.
fn assembled_source(entry: &Path) -> Result<String, Box<dyn std::error::Error>> {
    Ok(assembled(entry)?.source)
}

/// [`assembled_source`] with the closure's `SourceMap`, for the emitters that can fail at a line. The
/// program passes the front end first, so a rejected program reads the same on every backend.
fn assembled(entry: &Path) -> Result<crate::closure::Closure, Box<dyn std::error::Error>> {
    let dep_roots = dep_roots_for_entry(entry);
    crate::diagnostics::front_end(entry, &dep_roots)?;
    Ok(crate::closure::assemble_closure_with_deps(entry, dep_roots)
        .map_err(|e| format!("could not assemble project: {e}"))?)
}

/// A backend's refusal, rendered: `<what> failed:` then the diagnostic at the author's
/// `file:line:col`, placed through the closure the backend compiled.
fn failed(
    what: &str,
    pe: &logicaffeine_compile::error::ParseError,
    closure: &crate::closure::Closure,
    entry: &Path,
) -> Box<dyn std::error::Error> {
    let at =
        crate::diagnostics::Placed::locate(pe.span, &closure.source, &closure.source_map, entry);
    ui::CliError::new(format!(
        "{what} failed:\n{}",
        crate::diagnostics::render_parse_error(pe, &at)
    ))
    .into()
}

/// The declared dependency roots for an `entry` file, resolved from the enclosing
/// project's manifest. Empty when the entry is a standalone file with no project,
/// the manifest declares no linkable deps, OR a declaration can't be resolved — so
/// a bare-file `largo emit` is unchanged and a resolution problem never crashes the
/// emit. A floating git rev / missing path still fails loudly on the resolving
/// commands (`check`/`build`); emit degrades to the in-project closure.
pub(crate) fn dep_roots_for_entry(entry: &Path) -> Vec<logicaffeine_compile::loader::DepRoot> {
    let Some(root) = crate::project::build::find_project_root(entry) else {
        return Vec::new();
    };
    let Ok(manifest) = Manifest::load(&root) else {
        return Vec::new();
    };
    crate::project::deps::resolve_dep_roots(&root, &manifest).unwrap_or_default()
}

/// The source file to emit from: an explicit FILE, or the project entry
/// (with the standard `.md` fallback).
fn resolve_entry(file: Option<PathBuf>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match file {
        Some(f) => Ok(f),
        None => {
            let root = require_project_root()?;
            let manifest = Manifest::load(&root)?;
            Ok(crate::commands::resolve_entry_path(&root, &manifest)?)
        }
    }
}

/// Print emitted code to stdout, or write it to `-o` (stdout stays quiet).
/// The value runtime emitted C links when the exact extension reaches it: with `-o`, its archive written beside
/// the C and the command that links the two; on stdout, which carries only the C, a note that `-o` writes it.
fn write_value_runtime(code: &str, output: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    use logicaffeine_compile::codegen_c::value_runtime;
    if !value_runtime::links(code) {
        return Ok(());
    }
    let Some(path) = output else {
        ui::note("This program links the exact extension's value runtime: emit with -o to write it beside the C");
        return Ok(());
    };
    let archive = value_runtime::archive()
        .map_err(|why| ui::CliError::new(format!("the exact extension's value runtime did not build: {why}")))?;
    let lib = path.with_file_name("liblogos_value_runtime.a");
    fs::copy(&archive.path, &lib)?;
    ui::info(format!("Wrote {} (the exact extension's value runtime)", lib.display()));
    ui::info(format!(
        "Link with: cc {} {} {} -lm",
        path.display(),
        lib.display(),
        archive.system_libs.join(" ")
    ));
    Ok(())
}

/// Write what the RV64GC assembly `asm`, written at `asm_path`, links beside it: its runtime shim, `logos_rt.c`, and
/// when the program reaches the exact extension, the extension's value runtime built for RISC-V,
/// `liblogos_value_runtime_riscv64.a`. Answers what a link line names after the assembly.
fn write_riscv_runtime(asm: &str, asm_path: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    use logicaffeine_compile::codegen_riscv;
    let runtime = codegen_riscv::runtime(asm).map_err(|why| {
        ui::CliError::new(format!("the exact extension's value runtime did not build for RISC-V: {why}"))
    })?;
    let rt = asm_path.with_file_name("logos_rt.c");
    fs::write(&rt, &runtime.c)?;
    ui::info(format!("Wrote {} (compile alongside the .s)", rt.display()));
    let mut line = vec![rt.display().to_string()];
    let mut args = runtime.link_args.into_iter();
    if codegen_riscv::links_value_runtime(asm) {
        let archive = args.next().expect("the value runtime's archive heads its link line");
        let lib = asm_path.with_file_name("liblogos_value_runtime_riscv64.a");
        fs::copy(&archive, &lib)?;
        ui::info(format!("Wrote {} (the exact extension's value runtime)", lib.display()));
        line.push(lib.display().to_string());
    }
    line.extend(args);
    Ok(line)
}

fn write_emitted(code: &str, output: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    match output {
        Some(path) => {
            fs::write(path, code)?;
            ui::info(format!("Wrote {}", path.display()));
        }
        None => println!("{code}"),
    }
    Ok(())
}

/// `largo emit wasm[-linked]`: FILE mode compiles a standalone `.lg` and
/// writes beside it (or at `-o`); project mode matches `build --emit wasm`
/// (`target/<name>.wasm`), with `-o` overriding the module path. The host
/// shim always lands beside the module, sharing its stem.
fn emit_wasm_target(
    file: Option<PathBuf>,
    output: Option<PathBuf>,
    linked: bool,
    dom: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (entry, default_wasm_path, theme) = match &file {
        // A standalone file has no project to declare `[web] theme = false` in.
        Some(f) => (
            f.clone(),
            f.with_extension("wasm"),
            logicaffeine_compile::web_host::ShellTheme::Default,
        ),
        None => {
            let root = require_project_root()?;
            let manifest = Manifest::load(&root)?;
            let entry = crate::commands::resolve_entry_path(&root, &manifest)?;
            let out_dir = root.join("target");
            fs::create_dir_all(&out_dir)?;
            let theme = manifest.shell_theme();
            (
                entry,
                out_dir.join(format!("{}.wasm", manifest.package.name)),
                theme,
            )
        }
    };
    let closure = assembled(&entry)?;
    let wasm = compile_wasm_bytes(&closure, &entry, linked)?;
    let wasm_path = output.unwrap_or(default_wasm_path);
    if let Some(parent) = wasm_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&wasm_path, &wasm)?;
    let stem = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("output path needs a file name")?
        .to_string();
    let mjs = wasm_path.with_extension("mjs");
    fs::write(&mjs, wasm_host_shim(&stem))?;
    if dom {
        // A browser host: the real DOM behind the `dom_*` imports, events wired to
        // the LOGOS trampoline. Load `<stem>.html` over HTTP (module scripts need it).
        let js = wasm_path.with_extension("js");
        fs::write(&js, browser_host_js(&stem))?;
        let html = wasm_path.with_extension("html");
        fs::write(
            &html,
            logicaffeine_compile::web_host::browser_host_html_themed(&stem, None, None, theme),
        )?;
        ui::info(format!(
            "Built {} [wasm DOM app, {} bytes]",
            wasm_path.display(),
            wasm.len()
        ));
        ui::info(format!(
            "Open it: serve {} and load {}",
            wasm_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .display(),
            html.display()
        ));
        return Ok(());
    }
    let tier = if linked {
        "wasm-linked, Rust runtime"
    } else {
        "wasm, no rustc"
    };
    ui::info(format!(
        "Built {} [{tier}, {} bytes]",
        wasm_path.display(),
        wasm.len()
    ));
    ui::info(format!("Run it: node {}", mjs.display()));
    Ok(())
}

// The browser host (HTML shell + DOM/signal runtime JS) is shared with the web
// Studio's live preview, so it lives in `logicaffeine_compile::web_host`. Re-exported
// here so `emit`/`serve` call sites read unchanged.
pub(crate) use logicaffeine_compile::web_host::{
    browser_host_html, browser_host_js, wasm_host_shim,
};

/// Compile an assembled closure to wasm bytes on the selected backend; a refusal is rendered at
/// the author's `file:line:col`.
pub(crate) fn compile_wasm_bytes(
    closure: &crate::closure::Closure,
    entry: &Path,
    linked: bool,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if linked {
        logicaffeine_compile::compile::compile_to_wasm_linked(&closure.source)
            .map_err(|e| failed("the linked wasm build", &e, closure, entry))
    } else {
        logicaffeine_compile::compile::compile_to_wasm(&closure.source)
            .map_err(|e| failed("the wasm build", &e, closure, entry))
    }
}

/// Compile the project entry DIRECTLY to a standalone `.wasm` via the built-in backend (no
/// rustc/cargo/wasm-bindgen), writing `target/<name>.wasm` and its `<name>.mjs` host shim. Returns
/// `(mjs_path, wasm_byte_len)`. Shared by `--emit wasm` on both `build` and `run`.
pub(crate) fn build_wasm_module(
    project_root: &std::path::Path,
    linked: bool,
) -> Result<(std::path::PathBuf, usize), Box<dyn std::error::Error>> {
    let manifest = Manifest::load(project_root)?;
    let entry_path = crate::commands::resolve_entry_path(project_root, &manifest)?;
    let closure = assembled(&entry_path)?;
    // `--emit wasm` is fully self-contained (no toolchain; integer overflow wraps per the i64
    // spec). `--emit wasm-linked` instead links the real Rust runtime with `rust-lld`, so an
    // overflowing integer expression computes the exact big number and a program the exact
    // extension reaches runs on its value runtime (matching the VM) — at the cost of needing the
    // Rust toolchain + a wasm32 build of the runtime.
    let wasm = compile_wasm_bytes(&closure, &entry_path, linked)?;
    let out_dir = project_root.join("target");
    fs::create_dir_all(&out_dir)?;
    let name = &manifest.package.name;
    fs::write(out_dir.join(format!("{name}.wasm")), &wasm)?;
    let mjs = out_dir.join(format!("{name}.mjs"));
    fs::write(&mjs, wasm_host_shim(name))?;
    Ok((mjs, wasm.len()))
}

/// `largo build --emit wasm[-linked]` — compile to a standalone `.wasm` + `.mjs` host shim.
pub(crate) fn emit_wasm_module(
    project_root: &std::path::Path,
    linked: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mjs, bytes) = build_wasm_module(project_root, linked)?;
    let wasm = mjs.with_extension("wasm");
    let tier = if linked {
        "wasm-linked, Rust runtime"
    } else {
        "wasm, no rustc"
    };
    ui::info(format!("Built {} [{tier}, {bytes} bytes]", wasm.display()));
    ui::info(format!("Run it: node {}", mjs.display()));
    Ok(())
}

/// `largo build --emit isa`: compile the entry to Tier-5 LOGOS Silicon ISA assembly — the register-VM
/// opcode set AS the native chip ISA (`MAKE_A_CHIP.md` §10.1), every instruction annotated with its
/// reversibility contract — written beside the entry as `<entry>.isa`. No rustc / cargo needed.
pub(crate) fn emit_isa_module(
    project_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::load(project_root)?;
    let entry_path = crate::commands::resolve_entry_path(project_root, &manifest)?;
    let closure = assembled(&entry_path)?;
    let asm = logicaffeine_compile::compile::compile_to_isa(&closure.source)
        .map_err(|e| failed("Silicon ISA emit", &e, &closure, &entry_path))?;
    let out = entry_path.with_extension("isa");
    let count = asm
        .lines()
        .filter(|l| l.trim_start().starts_with('L'))
        .count();
    fs::write(&out, &asm)?;
    ui::info(format!(
        "Built {} [Tier-5 Silicon ISA, {count} instructions]",
        out.display()
    ));
    Ok(())
}

/// `largo build --emit riscv`: compile the entry to RV64GC assembly (the LOGOS → RISC-V AOT tier),
/// written beside the entry as `<entry>.s`. Assemble+link it with a RISC-V GCC and run under qemu.
pub(crate) fn emit_riscv_module(
    project_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::load(project_root)?;
    let entry_path = crate::commands::resolve_entry_path(project_root, &manifest)?;
    let closure = assembled(&entry_path)?;
    let asm = logicaffeine_compile::compile::compile_to_riscv(&closure.source)
        .map_err(|e| failed("RISC-V emit", &e, &closure, &entry_path))?;
    let out = entry_path.with_extension("s");
    fs::write(&out, &asm)?;
    ui::info(format!("Built {} [RV64GC assembly]", out.display()));
    let line = write_riscv_runtime(&asm, &out)?;
    ui::info(format!(
        "Run it: riscv64-linux-gnu-gcc -O2 -static -o {stem} {} {} && qemu-riscv64 ./{stem}",
        out.display(),
        line.join(" "),
        stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("prog"),
    ));
    Ok(())
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/emit/emit_wasm_tests.rs"]
mod emit_wasm_tests;
