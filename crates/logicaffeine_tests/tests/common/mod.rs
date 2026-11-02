//! E2E Test Harness
//!
//! Provides utilities for compiling LOGOS source and running the generated Rust.
//! Also provides test helpers for parsing and snapshots.

use std::path::PathBuf;
use std::process::Command;

// ============================================================
// G14 — honest external-tool preflights
// ============================================================

/// The launcher scripts/d5005/fetch-verilator.sh installs; ask `require_tool("verilator")` first.
pub fn verilator() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/verilator/bin/verilator")
}

/// `verilog` with every module but those named in `keep` a black box of its ports, its header kept and its body
/// dropped: yosys then elaborates only the modules a check reads, where a column's lanes and processing elements take it
/// minutes.
#[allow(dead_code)]
pub fn stubbed(verilog: &str, keep: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = verilog;
    loop {
        let at = if rest.starts_with("module ") { Some(0) } else { rest.find("\nmodule ").map(|i| i + 1) };
        let Some(at) = at else { break };
        out.push_str(&rest[..at]);
        let module = &rest[at..];
        let end = module.find("endmodule").expect("each module ends") + "endmodule".len();
        let name: String = module["module ".len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if keep.contains(&name.as_str()) {
            out.push_str(&module[..end]);
        } else {
            let header = module.find(");").expect("each module's ports end") + 2;
            out.push_str("(* blackbox *)\n");
            out.push_str(&module[..header]);
            out.push_str("\nendmodule");
        }
        rest = &module[end..];
    }
    out.push_str(rest);
    out
}

/// Whether `/proc/self/pagemap` hides this process's physical frames: the entry of a page it has just written is
/// present with frame number 0. False where the page map cannot be read.
fn hidden_frames() -> bool {
    use std::os::unix::fs::FileExt;
    let page = Box::new([1u8; 4096]);
    let at = (page.as_ptr() as u64) / 4096 * 8;
    let mut entry = [0u8; 8];
    let Ok(map) = std::fs::File::open("/proc/self/pagemap") else { return false };
    if map.read_exact_at(&mut entry, at).is_err() {
        return false;
    }
    let entry = u64::from_le_bytes(entry);
    std::hint::black_box(&page);
    entry >> 63 == 1 && entry & ((1 << 55) - 1) == 0
}

/// Is `tool` present on PATH? Probes `<tool> --version`, then a POSIX
/// `command -v <tool>` (for tools without a `--version`). Pure — no env, no
/// side effects — so it is deterministic and parallel-safe.
pub fn tool_available(tool: &str) -> bool {
    // Named probes for "tools" that are not executables on PATH: the Tenstorrent simulator (a shared object
    // plus a built tt-metal tree) and a usable GPU device. Everything else is a PATH probe.
    match tool {
        "ttsim" => return ttsim::available(),
        // The public Plasticine chain: a fetched+built toolchain tree plus a reachable docker.
        "plasticine-toolchain" => return plasticine::ready(),
        // PyTorch in the Python the vendor-format locks run (`LOGOS_PYTHON`, default `python3`) — a
        // module, not a binary, so `import torch` is the question.
        "torch" => {
            return Command::new(python_for_tools())
                .args(["-c", "import torch"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        // The Hugging Face intake's oracle: a Python (`LOGOS_PYTHON`) with torch, transformers and safetensors to build
        // a family's tiny twin, the pinned llama.cpp converter that turns it into the GGUF the intake is held to, and
        // the families' tokenizers (`scripts/accel/fetch-hf-tokenizers.sh` into the gitignored `external/hf-tokenizers/`).
        "hf-twins" => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            return root.join("external/competitors/llama.cpp/convert_hf_to_gguf.py").is_file()
                && root.join("external/hf-tokenizers/SHA256SUMS").is_file()
                && Command::new(python_for_tools())
                    .args(["-c", "import torch, transformers, safetensors"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
        }
        // The published configurations of the releases the Samba book deploys (`scripts/accel/fetch-hf-configs.sh` into
        // the gitignored `external/hf-configs/`), read without their weights.
        "hf-configs" => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            return root.join("external/hf-configs/SHA256SUMS").is_file();
        }
        // The heads of published GGUFs (`scripts/accel/fetch-gguf-heads.sh` into the gitignored `external/gguf-heads/`):
        // what llama.cpp's converter wrote from checkpoints whose own code the intake does not run.
        "gguf-heads" => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            return root.join("external/gguf-heads/SHA256SUMS").is_file();
        }
        // The published Hugging Face checkpoints of the golden models (`models/<model>/hf`) and what converting them
        // takes: the pinned converter and a Python with torch and transformers.
        "hf-published" => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            return ["qwen3.5-0.8b", "gemma3-1b", "llama3.2-1b"].iter().all(|m| root.join("models").join(m).join("hf/config.json").is_file())
                && root.join("external/competitors/llama.cpp/convert_hf_to_gguf.py").is_file()
                && Command::new(python_for_tools())
                    .args(["-c", "import torch, transformers, safetensors"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
        }
        // Qwen3.8-Flash-Next's tensor table: its configuration, every shard's safetensors header and its integer
        // tensors (`scripts/accel/qwen4exp-headers.py` reads them by range request into
        // `external/papers/qwen38-flash-next-headers/`).
        "qwen4exp-headers" => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../external/papers");
            let dir = root.join("qwen38-flash-next-headers");
            let headers = std::fs::read_dir(&dir)
                .map(|d| d.filter(|e| e.as_ref().is_ok_and(|e| e.file_name().to_string_lossy().ends_with(".header.json"))).count())
                .unwrap_or(0);
            return headers == 131
                && dir.join("constants.json").is_file()
                && root.join("hf-qwen3.8-flash-next-config.json").is_file();
        }
        // The lanes board the Samba book's agents report on, named by `SAMBA_LANES` (it lives outside the public tree).
        "samba-lanes" => return std::env::var_os("SAMBA_LANES").is_some_and(|p| std::path::Path::new(&p).is_file()),
        // The captures of the Samba book's sources (`scripts/accel/samba-papers.py`): every capture the pins table names.
        "samba-papers" => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let Ok(table) = std::fs::read_to_string(root.join("benchmarks/samba_papers.tsv")) else { return false };
            return table.lines().skip(1).filter(|l| !l.trim().is_empty()).all(|l| {
                let cells: Vec<&str> = l.split('\t').collect();
                cells.len() == 10 && root.join(cells[7]).is_file() && root.join(cells[9]).is_file()
            });
        }
        // The Unicode Character Database files the Hugging Face tokenizer port's tables are generated from
        // (`scripts/accel/fetch-unicode-data.sh`), at the two versions `tokenizers` reads.
        "unicode-data" => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../external/unicode");
            return ["16.0.0", "9.0.0"].iter().all(|v| root.join(v).join("SHA256SUMS").is_file());
        }
        "gpu" => return std::path::Path::new("/dev/nvidia0").exists(),
        // A page map that hides this process's physical frames, as Linux hides them from every process without
        // CAP_SYS_ADMIN: a mapped page whose entry is present reads frame 0.
        "hidden-frames" => return hidden_frames(),
        // The large inputs kept out of the repository — the Lean exports the Arena heavy cases and the
        // NbE differential read (Init, Std, cslib, Mathlib, …) — in the checkout beside the workspace,
        // which a clone does not carry.
        "reference-kernels" => {
            return reference_kernels_at(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        }
        // The pinned sources of the inference stacks the fusion-parity ledger cites, fetched into the gitignored
        // `external/competitors/`, which a clone does not carry.
        "competitor-sources" => {
            return competitor_sources_at(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        }
        // The pinned compute-sanitizer `scripts/accel/fetch-cuda-sanitizer.sh` unpacks into the gitignored
        // `external/cuda-sanitizer/`, which a clone does not carry.
        "cuda-sanitizer" => return cuda_sanitizer().is_file(),
        // The Qwen3-8B Q8_0 checkpoint under the gitignored `models/`, which a clone does not carry.
        "qwen3-8b-checkpoint" => return qwen3_8b_checkpoint().is_file(),
        // A checkpoint downloaded into the gitignored `models/<dir>/` (`checkpoint:<dir>`, or
        // `checkpoint:<dir>/<file>.gguf` for one of several there), which a clone does not carry.
        t if t.starts_with("checkpoint:") => return checkpoint(&t["checkpoint:".len()..]).is_some(),
        // The pinned llama.cpp built in place (`external/competitors/build-llama.sh`): its headers, `libllama` and
        // its bundled vocabulary corpora, the oracle the tokenizer locks link against.
        "llama-cpp-build" => return llama_cpp_build_at(&llama_cpp_root()),
        // The images deployed to the FPGA boards — `benchmarks/silicon/*.bit`, each beside the `.build.txt`
        // recipe its build wrote — kept out of the repository by `.gitignore`, which a clone does not carry.
        "fpga-images" => {
            return fpga_images_at(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        }
        // The decode boards' images — the bitstreams `benchmarks/silicon/decode_images.json` and
        // `decode_split_images.json` name — kept out of the repository by `.gitignore`, which a clone does not carry.
        "decode-images" => {
            return decode_images_at(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        }
        // The nextpnr-xilinx checkout scripts/setup-openxc7.sh clones into the gitignored `tools/openxc7/nextpnr-xilinx`,
        // the source the repository's patches apply to, which a clone does not carry.
        "openxc7-source" => {
            return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/openxc7/nextpnr-xilinx/.git").exists()
        }
        // The RTL the comparative-synthesis bench's third-party tools generated (`benchmarks/silicon/qor/rtl/`,
        // regenerated by scripts/qor/build-qor-candidate.py under the tools' own licences), which a clone does not
        // carry.
        "qor-rtl" => {
            return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/qor/rtl").is_dir()
        }
        // The LiteX environment scripts/qor/fetch-qor-tools.py installs (`tools/qor/litex-venv`), the one LiteDRAM's
        // controller is generated in, which a clone does not carry.
        "litex" => {
            return Command::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/qor/litex-venv/bin/python"))
                .args(["-c", "import litedram, migen"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        // The D5005 FIM's PIM sources and pyslang, which scripts/d5005/fetch-d5005-tools.sh unpacks and installs into
        // the gitignored `tools/d5005/` (`pr-template`, `venv`), which a clone does not carry.
        "d5005-pim" => {
            let t = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/d5005");
            return t.join("pr-template/hw/lib/build/platform/ofs_plat_if/sim/platform_if_addenda.txt").is_file()
                && Command::new(t.join("venv/bin/python"))
                    .args(["-c", "import pyslang"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
        }
        // Verilator, which scripts/d5005/fetch-verilator.sh unpacks with its launcher into the gitignored
        // `tools/verilator/`, which a clone does not carry.
        "verilator" => {
            return Command::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/verilator/bin/verilator"))
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        // OPAE's platform scripts, the AFU JSON schema and the C headers from the same script's unpack
        // (`tools/d5005/opae-sdk`).
        "opae-sdk" => {
            let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/d5005/opae-sdk");
            return sdk.join("platforms/platmgr/tools/rtl_src_config.py").is_file() && sdk.join("include/opae/fpga.h").is_file()
        }
        // The tools a Sky130 power figure is read with — the OpenSTA image scripts/opensta/build-opensta.sh builds,
        // the pinned OpenLane image, the sky130 library and its Verilog models, iverilog and yosys — which a clone
        // does not carry.
        "opensta-power" => {
            return opensta_power_ready(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        }
        // The machine-local EDA tree the silicon evidence lives in — OpenLane with this repository's hardened
        // designs and their run directories, and the Caravel harness checkouts — which a clone does not carry.
        "eda-tools" => {
            return eda_tools_at(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        }
        // The private-material guard: its program and lists, installed into this checkout's git directory from the
        // private workspace, which a clone does not carry.
        "private-guard" => {
            return private_guard_at(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
                .is_some()
        }
        // The LINKED wasm tier needs a wasm32 std to link `logicaffeine_base` against, which is a
        // rustup TARGET rather than an executable on PATH.
        "wasm32" => return rust_target_installed("wasm32-unknown-unknown"),
        // The board's compiler is built for the hart's Linux ABI: a rustup target with musl's static runtime.
        "riscv64-musl" => return rust_target_installed("riscv64gc-unknown-linux-musl"),
        _ => {}
    }
    let via_version = Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    via_version
        || Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {tool}"))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Is the standard library for the rustc `triple` installed? `rustc --print target-libdir` names the directory
/// whether or not the target is installed, so the question is whether that directory holds a `libstd` rlib.
pub fn rust_target_installed(triple: &str) -> bool {
    let Ok(out) = Command::new("rustc")
        .args(["--print", "target-libdir", "--target", triple])
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with("libstd-") && name.ends_with(".rlib")
            })
        })
        .unwrap_or(false)
}

/// Are the tools a Sky130 power figure is read with at hand for the checkout rooted at `root`: the image
/// `scripts/opensta/build-opensta.sh --name` names and the pinned OpenLane image both built, the sky130 HD library and
/// its Verilog models under `~/.ciel/sky130A`, and iverilog and yosys? The question behind
/// `tool_available("opensta-power")`.
pub fn opensta_power_ready(root: &std::path::Path) -> bool {
    let built = |image: &str| {
        Command::new("docker")
            .args(["image", "inspect", image])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let opensta = Command::new("bash")
        .arg(root.join("scripts/opensta/build-opensta.sh"))
        .arg("--name")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let hd = std::env::var("HOME").map(|h| PathBuf::from(h).join(".ciel/sky130A/libs.ref/sky130_fd_sc_hd"));
    opensta.is_some_and(|image| built(&image))
        && built("ghcr.io/the-openroad-project/openlane:ff5509f65b17bfa4068d5336495ab1718987ff69")
        && hd.is_ok_and(|hd| hd.join("lib").is_dir() && hd.join("verilog").is_dir())
        && ["iverilog", "yosys"].iter().all(|t| tool_available(t))
}

/// The Python interpreter the vendor-format locks drive: `LOGOS_PYTHON` when set (a venv with torch), else
/// `python3` from PATH.
pub fn python_for_tools() -> String {
    std::env::var("LOGOS_PYTHON")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "python3".to_string())
}

/// Does the checkout rooted at `root` carry the `reference-kernels/` input tree? The question behind
/// `tool_available("reference-kernels")`, taking the root so its answer is lockable both ways.
pub fn reference_kernels_at(root: &std::path::Path) -> bool {
    root.join("reference-kernels").is_dir()
}

/// The local Qwen3-8B Q8_0 checkpoint (`models/` is gitignored) — the question behind
/// `tool_available("qwen3-8b-checkpoint")`, and the path the locks that read it open.
pub fn qwen3_8b_checkpoint() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/qwen3-8b/Qwen_Qwen3-8B-Q8_0.gguf")
}

/// The checkpoint behind `tool_available("checkpoint:<name>")`: the GGUF `models/<dir>/<file>.gguf` names, or the
/// one GGUF under `models/<dir>/`.
pub fn checkpoint(name: &str) -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models").join(name);
    if name.ends_with(".gguf") {
        return d.is_file().then_some(d);
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(&d)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "gguf"))
        .collect();
    found.sort();
    (found.len() == 1).then(|| found.remove(0))
}

/// The pinned compute-sanitizer (tool 2025.1) under the gitignored `external/cuda-sanitizer/`
/// (`scripts/accel/fetch-cuda-sanitizer.sh`): the race, barrier and memory checker the lane's `cp.async` kernels run
/// under.
pub fn cuda_sanitizer() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../external/cuda-sanitizer/cuda_sanitizer_api-linux-x86_64-12.8.93-archive/compute-sanitizer/compute-sanitizer")
}

/// The pinned llama.cpp tree under the gitignored `external/competitors/`.
pub fn llama_cpp_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../external/competitors/llama.cpp")
}

/// Is the llama.cpp tree at `root` built: its public header, `libllama`, and the vocabulary corpora under
/// `models/`? The question behind `tool_available("llama-cpp-build")`.
pub fn llama_cpp_build_at(root: &std::path::Path) -> bool {
    root.join("include/llama.h").is_file()
        && root.join("build/bin/libllama.so").is_file()
        && root.join("models/ggml-vocab-qwen2.gguf").is_file()
}

/// Does the checkout rooted at `root` hold the competitor sources `benchmarks/fusion_parity.json` cites: the pin
/// table, and for every pinned stack its unpacked tree or its file listing (`<stack>.paths.txt`)? The question
/// behind `tool_available("competitor-sources")`.
pub fn competitor_sources_at(root: &std::path::Path) -> bool {
    let dir = root.join("external/competitors");
    let Ok(pins) = std::fs::read_to_string(dir.join("PINS.tsv")) else {
        return false;
    };
    let stacks: Vec<&str> = pins
        .lines()
        .skip(1)
        .filter_map(|l| l.split('\t').next())
        .collect();
    !stacks.is_empty()
        && stacks
            .iter()
            .all(|s| dir.join(s).is_dir() || dir.join(format!("{s}.paths.txt")).is_file())
}

/// Does the checkout rooted at `root` carry the machine-local EDA tree under `tools/`: OpenLane holding at least
/// one of this repository's `logos_*` designs, and the `caravel_user_project` and `caravel` checkouts? The
/// question behind `tool_available("eda-tools")`, taking the root so its answer is lockable both ways.
/// Whether the checkout rooted at `root` holds the board images the kit and flash locks read: the WPE engine's
/// `wpe_soc.bit` and the build recipe beside it. The question behind `tool_available("fpga-images")`.
pub fn fpga_images_at(root: &std::path::Path) -> bool {
    let silicon = root.join("benchmarks/silicon");
    silicon.join("wpe_soc.bit").is_file() && silicon.join("wpe_soc.bit.build.txt").is_file()
}

/// The ledgers that name the decode boards' images, under `benchmarks/silicon/`.
pub const DECODE_IMAGE_LEDGERS: [&str; 2] = ["decode_images.json", "decode_split_images.json"];

/// Every bitstream a decode ledger names: each `bitstream` it holds, at any depth.
pub fn decode_ledger_bitstreams(ledger: &serde_json::Value) -> Vec<String> {
    fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(serde_json::Value::String(b)) = m.get("bitstream") {
                    out.push(b.clone());
                }
                m.values().for_each(|x| walk(x, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(ledger, &mut out);
    out
}

/// Whether the checkout rooted at `root` holds the decode boards' images: any bitstream a decode ledger names. A checkout
/// holding one is held to every one, so an image lost from a checkout that has the others fails rather than skips. The
/// question behind `tool_available("decode-images")`.
pub fn decode_images_at(root: &std::path::Path) -> bool {
    let silicon = root.join("benchmarks/silicon");
    DECODE_IMAGE_LEDGERS.iter().any(|ledger| {
        std::fs::read_to_string(silicon.join(ledger))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .is_some_and(|j| decode_ledger_bitstreams(&j).iter().any(|b| silicon.join(b).is_file()))
    })
}

pub fn eda_tools_at(root: &std::path::Path) -> bool {
    let tools = root.join("tools");
    let designs = std::fs::read_dir(tools.join("OpenLane/designs"))
        .map(|d| {
            d.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("logos_") && e.path().is_dir())
        })
        .unwrap_or(false);
    designs && tools.join("caravel_user_project").is_dir() && tools.join("caravel").is_dir()
}

/// The private-material guard installed for the checkout rooted at `root`: `<git common dir>/info/private-guard/`
/// holding its program, its path list and its denylist, or `None`. The question behind
/// `tool_available("private-guard")`, taking the root so its answer is lockable both ways.
pub fn private_guard_at(root: &std::path::Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()).join("info/private-guard");
    ["private-guard.py", "private-paths.txt", "denylist.src"]
        .iter()
        .all(|f| dir.join(f).is_file())
        .then_some(dir)
}

/// Does the `LOGOS_REQUIRE_TOOLS` spec demand `tool`? `"1"` or `"all"` requires
/// EVERY tool; otherwise the spec is a comma/space list and only its named tools
/// are required. Pure (spec passed in) so the requirement policy is unit-lockable.
pub fn tool_required_by(spec: &str, tool: &str) -> bool {
    let s = spec.trim();
    s == "1"
        || s.eq_ignore_ascii_case("all")
        || s.split([',', ' ', '\t'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .any(|t| t == tool)
}

/// The requirement a runner's one `export LOGOS_REQUIRE_TOOLS=` gives a caller that set none.
#[allow(dead_code)]
pub fn required_by_runner(script: &str, name: &str) -> Option<String> {
    let exports: Vec<&str> = script
        .lines()
        .filter(|l| l.trim_start().starts_with("export LOGOS_REQUIRE_TOOLS="))
        .collect();
    assert!(
        exports.len() <= 1,
        "{name} sets LOGOS_REQUIRE_TOOLS more than once: {exports:#?}"
    );
    let value = exports
        .first()?
        .trim_start()
        .trim_start_matches("export LOGOS_REQUIRE_TOOLS=")
        .trim_matches('"');
    let value = value.replace("${LOGOS_REQUIRE_TOOLS:+$LOGOS_REQUIRE_TOOLS,}", "");
    Some(match value.strip_prefix("${LOGOS_REQUIRE_TOOLS:-") {
        Some(rest) => rest.trim_end_matches('}').to_string(),
        None => value,
    })
}

/// The exact machine-readable marker a tool-skip emits — the greppable line a CI
/// budget counts. Pure (no env, no IO) so the marker FORMAT is unit-lockable.
pub fn tool_skip_marker(tool: &str) -> String {
    format!("LOGOS_TOOL_SKIP: {tool}")
}

/// Preflight `tool` against an EXPLICIT requirement spec (the parallel-safe core
/// of [`require_tool`]). If present → `true`. If absent and REQUIRED by the spec
/// → PANIC (the release gate fails loud, never a silent SKIP-as-PASS). If absent
/// and not required → emit the machine-readable [`tool_skip_marker`] line
/// (a CI budget greps these) and return `false` so the caller can bail cleanly.
pub fn require_tool_strict(tool: &str, require_spec: &str) -> bool {
    if tool_available(tool) {
        return true;
    }
    if tool_required_by(require_spec, tool) {
        panic!(
            "required external tool '{tool}' is not available, but LOGOS_REQUIRE_TOOLS \
             demands it — the selected gate fails loud (never a silent SKIP-as-PASS)"
        );
    }
    eprintln!("{}", tool_skip_marker(tool));
    false
}

/// Preflight `tool`, reading the requirement from the `LOGOS_REQUIRE_TOOLS`
/// environment variable (unset = nothing required, the dev-box default). Returns
/// whether the caller may proceed; a `false` means "skip this test, honestly
/// recorded". Replaces the dishonest `eprintln!("SKIP …"); return;` idiom.
pub fn require_tool(tool: &str) -> bool {
    let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
    require_tool_strict(tool, &spec)
}

/// Preflight a usable GPU device (`/dev/nvidia0`), through the same honest seam as `require_tool`:
/// `LOGOS_REQUIRE_TOOLS=gpu` turns an absent device into a loud failure.
pub fn require_gpu() -> bool {
    require_tool("gpu")
}

pub mod hf_twins;
pub mod plasticine;
pub mod ttsim;

pub use logicaffeine_compile::compile::{
    compile_program_full, compile_program_full_deterministic, compile_to_rust,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

// ============================================================
// Parse Helper - replaces the logos::parse! macro
// ============================================================

use logicaffeine_base::{Arena, Interner, Symbol};
use logicaffeine_language::analysis::TypeRegistry;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::{LogicExpr, NounPhrase, Term, ThematicRole};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::view::{ExprView, Resolve};
use logicaffeine_language::{Lexer, Parser};

/// Parse a sentence and return its ExprView representation.
/// This replaces the `parse!` macro with a proper function using tier crates.
pub fn parse_to_view(input: &str) -> ExprView<'static> {
    let interner: &'static mut Interner = Box::leak(Box::new(Interner::new()));
    let world_state: &'static mut WorldState = Box::leak(Box::new(WorldState::new()));
    let expr_arena: &'static Arena<LogicExpr> = Box::leak(Box::new(Arena::new()));
    let term_arena: &'static Arena<Term> = Box::leak(Box::new(Arena::new()));
    let np_arena: &'static Arena<NounPhrase> = Box::leak(Box::new(Arena::new()));
    let sym_arena: &'static Arena<Symbol> = Box::leak(Box::new(Arena::new()));
    let role_arena: &'static Arena<(ThematicRole, Term)> = Box::leak(Box::new(Arena::new()));
    let pp_arena: &'static Arena<&'static LogicExpr> = Box::leak(Box::new(Arena::new()));

    let ctx = AstContext::new(
        expr_arena, term_arena, np_arena, sym_arena, role_arena, pp_arena,
    );

    let mut lexer = Lexer::new(input, interner);
    let tokens = lexer.tokenize();

    let type_registry = TypeRegistry::default();
    let mut parser = Parser::new(tokens, world_state, interner, ctx, type_registry);

    let ast = parser.parse().unwrap();
    ast.resolve(interner)
}

/// Macro for convenient parsing in tests - delegates to parse_to_view
#[macro_export]
macro_rules! parse {
    ($input:expr) => {
        $crate::common::parse_to_view($input)
    };
}

// ============================================================
// Snapshot Testing
// ============================================================

/// Assert that actual output matches a stored snapshot file.
/// If UPDATE_SNAPSHOTS=1 is set, updates the snapshot instead.
#[macro_export]
macro_rules! assert_snapshot {
    ($name:expr, $actual:expr) => {{
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
        let snapshot_dir = std::path::Path::new(&manifest_dir)
            .join("tests")
            .join("snapshots");
        let snapshot_path = snapshot_dir.join(format!("{}.txt", $name));

        if !snapshot_dir.exists() {
            std::fs::create_dir_all(&snapshot_dir).expect("Failed to create snapshot dir");
        }

        let actual_str = $actual.trim();
        let force_update = std::env::var("UPDATE_SNAPSHOTS").is_ok();

        if force_update || !snapshot_path.exists() {
            std::fs::write(&snapshot_path, actual_str).expect("Failed to write snapshot");
            println!("Snapshot created/updated: {:?}", snapshot_path);
        } else {
            let expected =
                std::fs::read_to_string(&snapshot_path).expect("Failed to read snapshot");
            let expected_str = expected.trim();

            if actual_str != expected_str {
                panic!(
                    "\nSnapshot Mismatch: {}\n\nExpected:\n{}\n\nActual:\n{}\n\n\
                    Run `UPDATE_SNAPSHOTS=1 cargo test` to update.\n",
                    $name, expected_str, actual_str
                );
            }
        }
    }};
}

static SHARED_TARGET_DIR: OnceLock<PathBuf> = OnceLock::new();
static COMPILE_COUNTER: AtomicU64 = AtomicU64::new(0);
static RUN_ID: OnceLock<u64> = OnceLock::new();

fn get_run_id() -> u64 {
    *RUN_ID.get_or_init(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
    })
}

/// Shard count for the e2e cargo cache under nextest. Keep in sync with the
/// `e2e-subprocess` group's max-threads in .config/nextest.toml: group slots are
/// 0..max-threads, so concurrently running tests always land in distinct shards
/// and never contend on Cargo's build-dir lock.
const E2E_TARGET_SHARDS: u64 = 12;

fn get_shared_target_dir() -> &'static PathBuf {
    SHARED_TARGET_DIR.get_or_init(|| {
        let shard = std::env::var("NEXTEST_TEST_GROUP_SLOT")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| {
                std::env::var("NEXTEST_TEST_GLOBAL_SLOT")
                    .ok()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(|n| n % E2E_TARGET_SHARDS)
            });
        let dir = match shard {
            Some(s) => std::env::temp_dir().join(format!("logos_e2e_cache_{s}")),
            None => std::env::temp_dir().join("logos_e2e_cache"),
        };
        std::fs::create_dir_all(&dir).expect("Failed to create shared target dir");
        // Safety valve: per-project artifacts are pruned after every test
        // (see prune_project_artifacts), so the cache holds only shared
        // dependency builds and stays a few GB. Anything that slips through
        // (interrupted tests, binaries built before the pruning landed) is
        // collected here BY AGE — generated-project artifacts older than an
        // hour cannot belong to a live test, while a whole-directory wipe
        // would delete dependency rmeta out from under CONCURRENT builds in
        // the same shard (global-slot tests share shards modulo the count).
        let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for sub in [
            "debug",
            "debug/deps",
            "debug/incremental",
            "debug/.fingerprint",
        ] {
            let Ok(rd) = std::fs::read_dir(dir.join(sub)) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name();
                let n = name.to_string_lossy();
                let generated = n.starts_with("logos_e2e_")
                    || n.starts_with("logos_clink_")
                    || n.starts_with("logos_run_")
                    || n.starts_with("liblogos_e2e_")
                    || n.starts_with("liblogos_clink_");
                if !generated {
                    continue;
                }
                let old_enough = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .map(|t| t < stale)
                    .unwrap_or(false);
                if old_enough {
                    let path = e.path();
                    let _ = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                }
            }
        }
        dir
    })
}

/// Delete one generated project's artifacts from the shared cache,
/// KEEPING the third-party dependency builds (the entire speed win).
/// Every project has a unique name, so without this the cache accretes a
/// binary + deps/incremental/fingerprint entries per test forever —
/// hundreds of GB per shard across repeated suite runs.
fn prune_project_artifacts(target: &std::path::Path, pkg_name: &str) {
    let stem = pkg_name.replace('-', "_");
    let debug = target.join("debug");
    let _ = std::fs::remove_file(debug.join(pkg_name));
    let _ = std::fs::remove_file(debug.join(format!("{pkg_name}.d")));
    let _ = std::fs::remove_file(debug.join(format!("lib{stem}.a")));
    let _ = std::fs::remove_file(debug.join(format!("lib{stem}.so")));
    for sub in ["deps", "incremental", ".fingerprint", "build"] {
        let Ok(rd) = std::fs::read_dir(debug.join(sub)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if n.starts_with(&format!("{stem}-"))
                || n.starts_with(&format!("lib{stem}-"))
                || n.starts_with(&format!("{pkg_name}-"))
            {
                let path = e.path();
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
            }
        }
    }
}

/// Seed a generated project with the workspace's Cargo.lock so dependency
/// resolution is pinned to the exact versions the workspace tests against.
/// Without this every project re-resolves from the registry cache, and a newly
/// published transitive version (e.g. `time` 0.3.48 vs `rcgen` 0.11.3) can
/// break every e2e build mid-run.
fn seed_lockfile(workspace_root: &std::path::Path, project_dir: &std::path::Path) {
    std::fs::copy(
        workspace_root.join("Cargo.lock"),
        project_dir.join("Cargo.lock"),
    )
    .expect("Failed to seed Cargo.lock into generated project");
}

pub struct E2EResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
}

pub struct CompileResult {
    pub binary_path: std::path::PathBuf,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
    pub _temp_dir: tempfile::TempDir, // Keep alive to prevent cleanup
}

impl Drop for CompileResult {
    fn drop(&mut self) {
        // The test has finished with the binary — drop its artifacts from
        // the shared cache (dependency builds stay).
        if let (Some(dir), Some(name)) = (self.binary_path.parent(), self.binary_path.file_name()) {
            if let Some(target) = dir.parent() {
                prune_project_artifacts(target, &name.to_string_lossy());
            }
        }
    }
}

/// Format user-declared dependencies as Cargo.toml lines.
fn format_user_deps(deps: &[logicaffeine_compile::CrateDependency]) -> String {
    let mut out = String::new();
    for dep in deps {
        if dep.features.is_empty() {
            out.push_str(&format!("{} = \"{}\"\n", dep.name, dep.version));
        } else {
            let feats = dep
                .features
                .iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "{} = {{ version = \"{}\", features = [{}] }}\n",
                dep.name, dep.version, feats
            ));
        }
    }
    out
}

/// Compile LOGOS source to a binary without running it.
/// Returns the path to the compiled binary.
pub fn compile_logos(source: &str) -> CompileResult {
    // 1. Compile LOGOS to Rust (with dependency extraction)
    let compile_output = match compile_program_full(source) {
        Ok(out) => out,
        Err(e) => {
            return CompileResult {
                binary_path: std::path::PathBuf::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
                _temp_dir: tempfile::tempdir().unwrap(),
            };
        }
    };
    let rust_code = compile_output.rust_code;
    let user_deps = format_user_deps(&compile_output.dependencies);

    // 2. Create temp project
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    // 3. Write Cargo.toml with unique package name (includes run ID to avoid stale binary issues)
    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_e2e_{}_{}", get_run_id(), pkg_id);
    // CARGO_MANIFEST_DIR points to crates/logicaffeine_tests, go up twice to get workspace root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
memchr = "2"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    // 4. Build (but don't run) - use shared target dir for caching
    //    Try --offline first, fall back to online if cache isn't warm.
    let output = Command::new("cargo")
        .args(["build", "--quiet", "--offline"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo build");
    let output = if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("--offline")
    {
        Command::new("cargo")
            .args(["build", "--quiet"])
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .output()
            .expect("cargo build")
    } else {
        output
    };

    let binary_path = get_shared_target_dir().join(format!("debug/{}", pkg_name));

    CompileResult {
        binary_path,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        rust_code,
        _temp_dir: temp_dir,
    }
}

/// Compile LOGOS source to optimized LLVM IR (release profile) and return
/// the .ll text. The bounds-hint pass/fail check greps `_logos_main` for
/// `panic_bounds_check` in this output.
#[allow(dead_code)]
pub fn compile_logos_llvm_ir(source: &str) -> String {
    let compile_output = compile_program_full(source)
        .unwrap_or_else(|e| panic!("LOGOS compile error: {:?}\n\nSource:\n{}", e, source));
    let rust_code = compile_output.rust_code;
    let user_deps = format_user_deps(&compile_output.dependencies);

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_ir_{}_{}", get_run_id(), pkg_id);
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[profile.release]
codegen-units = 1

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
memchr = "2"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    let output = Command::new("cargo")
        .args([
            "rustc",
            "--release",
            "--quiet",
            "--offline",
            "--",
            "--emit=llvm-ir",
        ])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo rustc --emit=llvm-ir");
    let output = if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("--offline")
    {
        Command::new("cargo")
            .args(["rustc", "--release", "--quiet", "--", "--emit=llvm-ir"])
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .output()
            .expect("cargo rustc --emit=llvm-ir")
    } else {
        output
    };
    assert!(
        output.status.success(),
        "release IR build failed.\nstderr: {}\n\nGenerated Rust:\n{}",
        String::from_utf8_lossy(&output.stderr),
        rust_code
    );

    let deps_dir = get_shared_target_dir().join("release/deps");
    let prefix = pkg_name.replace('-', "_");
    let ll_path = std::fs::read_dir(&deps_dir)
        .expect("read deps dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            p.extension().map_or(false, |x| x == "ll")
                && p.file_stem()
                    .and_then(|s| s.to_str())
                    .map_or(false, |s| s.starts_with(&prefix))
        })
        .unwrap_or_else(|| panic!("no .ll emitted for {} in {}", prefix, deps_dir.display()));
    let ir = std::fs::read_to_string(&ll_path).expect("read .ll");
    let _ = std::fs::remove_file(&ll_path);
    prune_project_artifacts(get_shared_target_dir(), &pkg_name);
    ir
}

/// Extract one function's definition body from LLVM IR text by symbol
/// substring (e.g. "_logos_main").
#[allow(dead_code)]
pub fn llvm_ir_function<'a>(ir: &'a str, symbol: &str) -> &'a str {
    let def_start = ir
        .match_indices("\ndefine ")
        .find(|(i, _)| {
            let line_end = ir[*i + 1..]
                .find('\n')
                .map(|j| i + 1 + j)
                .unwrap_or(ir.len());
            ir[*i..line_end].contains(symbol)
        })
        .map(|(i, _)| i + 1)
        .unwrap_or_else(|| panic!("no define line containing `{}` in IR", symbol));
    let body_end = ir[def_start..]
        .find("\n}")
        .map(|j| def_start + j + 2)
        .unwrap_or(ir.len());
    &ir[def_start..body_end]
}

/// Compile LOGOS source and run the generated Rust, returning result.
pub fn run_logos(source: &str) -> E2EResult {
    run_logos_with_args(source, &[])
}

/// Like [`run_logos`] but forwards `prog_args` to the compiled binary's argv (after
/// `--`), so a benchmark program reading `item 2 of args()` receives its size. This is
/// the full-program AOT-native run (Logos → Rust → rustc → binary) the benchmark-corpus
/// gate uses to prove every benchmark runs correctly on compiled-native code.
pub fn run_logos_with_args(source: &str, prog_args: &[&str]) -> E2EResult {
    // 1. Compile LOGOS to Rust (with dependency extraction)
    let compile_output = match compile_program_full(source) {
        Ok(out) => out,
        Err(e) => {
            return E2EResult {
                stdout: String::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
            };
        }
    };
    run_rust_project(
        &compile_output.rust_code,
        &compile_output.dependencies,
        prog_args,
    )
}

/// Compile and run an already-emitted Rust program: write it into a fresh cargo
/// project (with the workspace crates as path deps and the pinned lockfile), then
/// `cargo run` it in the shared, sharded target cache. This is the single build-run
/// path shared by [`run_logos_with_args`] (single-source) and multi-FILE callers
/// that go through `compile_project` — the only way to prove the emitted Rust
/// actually rustc-compiles and executes, not just that it string-matches.
pub fn run_rust_project(
    rust_code: &str,
    dependencies: &[logicaffeine_compile::CrateDependency],
    prog_args: &[&str],
) -> E2EResult {
    let user_deps = format_user_deps(dependencies);

    // Create temp project
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    // Write Cargo.toml with unique package name (includes run ID to avoid stale binary issues)
    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_e2e_{}_{}", get_run_id(), pkg_id);
    // CARGO_MANIFEST_DIR points to crates/logicaffeine_tests, go up twice to get workspace root
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
memchr = "2"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), rust_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    // Run - use shared target dir for caching
    //    Try --offline first to avoid transient crates.io failures, fall back to online.
    //    Set RUST_MIN_STACK=64MB so deeply-recursive PE/cogen programs
    //    don't overflow on structural AST walks.
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--offline", "--"])
        .args(prog_args)
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .env("RUST_MIN_STACK", "268435456")
        .envs(bootstrap_child_env())
        .output()
        .expect("cargo run");
    // Fall back to online if --offline failed (e.g. first run, cache not warm)
    let output = if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("--offline")
    {
        Command::new("cargo")
            .args(["run", "--quiet", "--"])
            .args(prog_args)
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .env("RUST_MIN_STACK", "268435456")
            .envs(bootstrap_child_env())
            .output()
            .expect("cargo run")
    } else {
        output
    };

    prune_project_artifacts(get_shared_target_dir(), &pkg_name);

    E2EResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        rust_code: rust_code.to_string(),
    }
}

/// The environment a compiled program runs under: inside the bootstrap scope, reference semantics
/// (`LOGOS_VALUE_SEMANTICS=0`), as `compile.rs` runs the compiler's own programs; otherwise none.
fn bootstrap_child_env() -> Vec<(&'static str, &'static str)> {
    if logicaffeine_compile::semantics::collections::reference_scope_active() {
        vec![("LOGOS_VALUE_SEMANTICS", "0")]
    } else {
        Vec::new()
    }
}

/// [`run_logos`] for a bootstrap program — one of the compiler's own LOGOS programs (the partial
/// evaluator and its dialects, the self-interpreter, the decompiler, a projection's compiler),
/// whole or driving an encoded program: compiled and run inside the bootstrap scope, under
/// reference semantics, as `compile.rs` compiles and runs each of them (`bootstrap_scope_lock.rs`).
#[allow(dead_code)]
pub fn run_bootstrap_logos(source: &str) -> E2EResult {
    logicaffeine_compile::semantics::collections::with_reference_semantics(|| run_logos(source))
}

/// [`assert_exact_output`] for a bootstrap program, compiled and run as [`run_bootstrap_logos`] does.
#[allow(dead_code)]
pub fn assert_bootstrap_exact_output(source: &str, expected: &str) {
    logicaffeine_compile::semantics::collections::with_reference_semantics(|| {
        assert_exact_output(source, expected)
    })
}

/// [`run_logos`] for a user program in a test that also builds a bootstrap program — a residual a
/// projection produced, a target run directly — named apart so `bootstrap_scope_lock.rs` can tell
/// which of that test's runs are the bootstrap program's. It runs under value semantics.
#[allow(dead_code)]
pub fn run_user_logos(source: &str) -> E2EResult {
    run_logos(source)
}

/// [`assert_exact_output`] for a user program in a test that also builds a bootstrap program (see
/// [`run_user_logos`]).
#[allow(dead_code)]
pub fn assert_user_exact_output(source: &str, expected: &str) {
    assert_exact_output(source, expected)
}

/// Assert that LOGOS code runs and produces expected output.
pub fn assert_output(source: &str, expected: &str) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source, result.rust_code, result.stderr
    );
    assert!(
        result.stdout.trim().contains(expected),
        "Expected '{}' in output.\nGot: '{}'\n\nSource:\n{}\n\nGenerated Rust:\n{}",
        expected,
        result.stdout.trim(),
        source,
        result.rust_code
    );
}

/// Assert that LOGOS code runs and produces exactly the expected output (trimmed).
#[allow(dead_code)]
pub fn assert_exact_output(source: &str, expected: &str) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source, result.rust_code, result.stderr
    );
    assert_eq!(
        result.stdout.trim(),
        expected,
        "\nSource:\n{}\n\nGenerated Rust:\n{}",
        source,
        result.rust_code
    );
}

/// Assert that LOGOS code runs and produces exactly the expected lines (trimmed, line-by-line).
#[allow(dead_code)]
pub fn assert_output_lines(source: &str, expected_lines: &[&str]) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source, result.rust_code, result.stderr
    );
    let actual_lines: Vec<&str> = result.stdout.trim().lines().collect();
    assert_eq!(
        actual_lines.len(),
        expected_lines.len(),
        "Line count mismatch.\nExpected {} lines: {:?}\nGot {} lines: {:?}\n\nSource:\n{}\n\nGenerated Rust:\n{}",
        expected_lines.len(),
        expected_lines,
        actual_lines.len(),
        actual_lines,
        source,
        result.rust_code
    );
    for (i, (actual, expected)) in actual_lines.iter().zip(expected_lines.iter()).enumerate() {
        assert_eq!(
            actual.trim(),
            *expected,
            "Line {} mismatch.\nExpected: '{}'\nGot:      '{}'\n\nFull output:\n{}\n\nSource:\n{}\n\nGenerated Rust:\n{}",
            i + 1,
            expected,
            actual.trim(),
            result.stdout.trim(),
            source,
            result.rust_code
        );
    }
}

/// Assert that LOGOS code runs and output contains all specified substrings (order-independent).
/// Use for non-deterministic output (e.g., concurrent tasks).
#[allow(dead_code)]
pub fn assert_output_contains_all(source: &str, parts: &[&str]) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source, result.rust_code, result.stderr
    );
    let output = result.stdout.trim();
    for part in parts {
        assert!(
            output.contains(part),
            "Expected '{}' in output.\nGot: '{}'\n\nSource:\n{}\n\nGenerated Rust:\n{}",
            part,
            output,
            source,
            result.rust_code
        );
    }
}

/// Assert that LOGOS code runs successfully (no output check).
#[allow(dead_code)]
pub fn assert_runs(source: &str) {
    let result = run_logos(source);
    assert!(
        result.success,
        "Code should run.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        source, result.rust_code, result.stderr
    );
}

/// Assert that LOGOS code panics at runtime (e.g., debug_assert! failure).
#[allow(dead_code)]
pub fn assert_panics(source: &str, expected_msg: &str) {
    let result = run_logos(source);
    assert!(
        !result.success,
        "Code should panic but succeeded.\nSource:\n{}\n\nGenerated Rust:\n{}\n\nstdout: {}",
        source, result.rust_code, result.stdout
    );
    assert!(
        result.stderr.contains(expected_msg),
        "Expected '{}' in panic message.\nGot stderr:\n{}\n\nSource:\n{}\n\nGenerated Rust:\n{}",
        expected_msg,
        result.stderr,
        source,
        result.rust_code
    );
}

// ============================================================
// C Codegen E2E — compile LOGOS to C, build as a release C build does, run
// ============================================================

/// The flags a release C build compiles with (`largo build --release --emit c`): full optimization for this
/// CPU, link-time optimization, and no contraction of `a * b + c` into one fused multiply-add, whose single
/// rounding the VM's arithmetic never takes.
#[allow(dead_code)]
pub const C_RELEASE_FLAGS: &[&str] = &["-O3", "-march=native", "-flto", "-ffp-contract=off"];

/// The C compiler a build runs: `$CC`, else `cc`.
#[allow(dead_code)]
pub fn c_compiler() -> String {
    std::env::var("CC")
        .ok()
        .filter(|cc| !cc.trim().is_empty())
        .unwrap_or_else(|| "cc".to_string())
}

/// The C source `src` built into the program `bin` as a release C build builds it.
#[allow(dead_code)]
pub fn build_c_release(
    src: &std::path::Path,
    bin: &std::path::Path,
) -> std::io::Result<std::process::Output> {
    std::process::Command::new(c_compiler())
        .args(C_RELEASE_FLAGS)
        .arg("-o")
        .arg(bin)
        .arg(src)
        .arg("-lm")
        .output()
}

/// Compile LOGOS source to C, build it as a release C build does, run, and check exact output.
#[allow(dead_code)]
pub fn assert_c_output(source: &str, expected: &str) {
    use logicaffeine_compile::compile::compile_to_c;

    let c_code = compile_to_c(source).unwrap_or_else(|e| {
        panic!("LOGOS→C compile error: {:?}\n\nSource:\n{}", e, source);
    });

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let c_path = temp_dir.path().join("main.c");
    let bin_path = temp_dir.path().join("main");

    std::fs::write(&c_path, &c_code).expect("write C file");

    let compile = build_c_release(&c_path, &bin_path).expect("run the C compiler");

    assert!(
        compile.status.success(),
        "gcc should compile successfully.\nstderr: {}\n\nGenerated C:\n{}",
        String::from_utf8_lossy(&compile.stderr),
        c_code
    );

    let run = std::process::Command::new(&bin_path)
        .output()
        .expect("run binary");

    assert!(
        run.status.success(),
        "C binary should run successfully.\nstderr: {}\n\nGenerated C:\n{}",
        String::from_utf8_lossy(&run.stderr),
        c_code
    );

    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(
        stdout.trim(),
        expected,
        "\nSource:\n{}\n\nGenerated C:\n{}",
        source,
        c_code
    );
}

/// Compile LOGOS source to C, build it as a release C build does, run, and assert the binary TRAPS: it must exit
/// non-zero with `expected_stderr` on stderr. The C tier must fail-loud exactly where the VM and
/// interpreter do (`item k of m` on an absent key is a runtime error — a missing key requires a
/// `contains` guard or `counterGet`, never a silent default). Mirrors `assert_c_output` but inverts
/// the run-success check, so trap semantics are locked at the C tier the same way normal output is.
#[allow(dead_code)]
pub fn assert_c_traps(source: &str, expected_stderr: &str) {
    use logicaffeine_compile::compile::compile_to_c;

    let c_code = compile_to_c(source).unwrap_or_else(|e| {
        panic!("LOGOS→C compile error: {:?}\n\nSource:\n{}", e, source);
    });

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let c_path = temp_dir.path().join("main.c");
    let bin_path = temp_dir.path().join("main");

    std::fs::write(&c_path, &c_code).expect("write C file");

    let compile = build_c_release(&c_path, &bin_path).expect("run the C compiler");

    assert!(
        compile.status.success(),
        "gcc should compile successfully.\nstderr: {}\n\nGenerated C:\n{}",
        String::from_utf8_lossy(&compile.stderr),
        c_code
    );

    let run = std::process::Command::new(&bin_path)
        .output()
        .expect("run binary");

    assert!(
        !run.status.success(),
        "C binary should TRAP on the missing key but exited successfully.\nstdout: {}\n\nGenerated C:\n{}",
        String::from_utf8_lossy(&run.stdout),
        c_code
    );

    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains(expected_stderr),
        "Expected '{}' on stderr.\nGot stderr: {}\n\nSource:\n{}\n\nGenerated C:\n{}",
        expected_stderr,
        stderr,
        source,
        c_code
    );
}

// ============================================================
// C ABI Linkage Tests — compile Rust staticlib, link with C
// ============================================================

pub struct CLinkResult {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
    pub rust_code: String,
    pub c_code: String,
}

/// Compile LOGOS source to a staticlib, compile C code, link, and run.
/// The generated Rust is built as a staticlib, then a C file is compiled
/// and linked against it to verify the ABI works end-to-end.
#[allow(dead_code)]
pub fn compile_and_link_c(source: &str, c_code: &str) -> CLinkResult {
    // 1. Compile LOGOS to Rust
    let compile_output = match compile_program_full(source) {
        Ok(out) => out,
        Err(e) => {
            return CLinkResult {
                stdout: String::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
                c_code: c_code.to_string(),
            };
        }
    };
    let rust_code = compile_output.rust_code;
    let user_deps = format_user_deps(&compile_output.dependencies);

    // 2. Create temp project configured as staticlib
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();

    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_clink_{}_{}", get_run_id(), pkg_id);
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();

    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["staticlib"]

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
memchr = "2"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    // Strip fn main() from the generated Rust — staticlib doesn't need it
    let lib_code = strip_main_function(&rust_code);

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/lib.rs"), &lib_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    // 3. Build the staticlib — try offline first, fall back to online
    let build_output = Command::new("cargo")
        .args(["build", "--quiet", "--offline"])
        .current_dir(project_dir)
        .env("CARGO_TARGET_DIR", get_shared_target_dir())
        .output()
        .expect("cargo build");
    let build_output = if !build_output.status.success()
        && String::from_utf8_lossy(&build_output.stderr).contains("--offline")
    {
        Command::new("cargo")
            .args(["build", "--quiet"])
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .output()
            .expect("cargo build")
    } else {
        build_output
    };

    if !build_output.status.success() {
        return CLinkResult {
            stdout: String::new(),
            stderr: format!(
                "Rust staticlib build failed:\n{}\n\nLib code:\n{}",
                String::from_utf8_lossy(&build_output.stderr),
                lib_code
            ),
            success: false,
            rust_code: rust_code.clone(),
            c_code: c_code.to_string(),
        };
    }

    let lib_path =
        get_shared_target_dir().join(format!("debug/lib{}.a", pkg_name.replace('-', "_")));

    // 4. Write C code
    let c_file = project_dir.join("test.c");
    std::fs::write(&c_file, c_code).unwrap();

    // 5. Compile and link C code against the staticlib
    let binary_path = project_dir.join("test_binary");
    let mut cc_args = vec![
        "-Wall",
        c_file.to_str().unwrap(),
        lib_path.to_str().unwrap(),
        "-o",
        binary_path.to_str().unwrap(),
    ];

    #[cfg(target_os = "macos")]
    {
        cc_args.extend_from_slice(&[
            "-framework",
            "Security",
            "-framework",
            "CoreFoundation",
            "-framework",
            "SystemConfiguration",
            "-lSystem",
            "-lresolv",
            "-liconv",
        ]);
    }

    #[cfg(target_os = "linux")]
    {
        cc_args.extend_from_slice(&["-ldl", "-lpthread", "-lm"]);
    }

    let cc_output = Command::new("cc")
        .args(&cc_args)
        .output()
        .expect("cc command");

    if !cc_output.status.success() {
        return CLinkResult {
            stdout: String::new(),
            stderr: format!(
                "C linking failed:\n{}\n\nC code:\n{}\n\nLib path: {:?}",
                String::from_utf8_lossy(&cc_output.stderr),
                c_code,
                lib_path
            ),
            success: false,
            rust_code: rust_code.clone(),
            c_code: c_code.to_string(),
        };
    }

    // 6. Run the linked binary
    let run_output = Command::new(&binary_path)
        .output()
        .expect("run test binary");

    prune_project_artifacts(get_shared_target_dir(), &pkg_name);

    CLinkResult {
        stdout: String::from_utf8_lossy(&run_output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&run_output.stderr).to_string(),
        success: run_output.status.success(),
        rust_code,
        c_code: c_code.to_string(),
    }
}

/// Strip the `fn main() { ... }` block from generated Rust code.
/// Used when building as a staticlib (no main needed).
fn strip_main_function(code: &str) -> String {
    let lines: Vec<&str> = code.lines().collect();
    let mut result = Vec::new();
    let mut skip = false;
    let mut depth = 0i32;

    for line in &lines {
        if line.trim_start().starts_with("fn main()") {
            skip = true;
            depth = 0;
        }
        if skip {
            for ch in line.chars() {
                if ch == '{' {
                    depth += 1;
                }
                if ch == '}' {
                    depth -= 1;
                }
            }
            if depth <= 0 && line.contains('}') {
                skip = false;
            }
            continue;
        }
        result.push(*line);
    }

    result.join("\n")
}

// ============================================================
// Interpreter Tests (no Rust compilation)
// ============================================================

pub struct InterpreterTestResult {
    pub output: String,
    pub error: String,
    pub success: bool,
}

/// Run LOGOS source through the interpreter (no Rust compilation).
/// Uses futures::executor::block_on to run the async interpret_for_ui.
pub fn run_interpreter(source: &str) -> InterpreterTestResult {
    use futures::executor::block_on;
    use logicaffeine_compile::interpret_for_ui;

    let result = block_on(interpret_for_ui(source));
    let success = result.error.is_none();

    InterpreterTestResult {
        output: result.lines.join("\n"),
        error: result.error.unwrap_or_default(),
        success,
    }
}

/// Assert that LOGOS code produces exactly the expected output via the interpreter.
/// No Rust compilation — runs directly through interpret_for_ui.
#[allow(dead_code)]
pub fn assert_interpreter_output(source: &str, expected: &str) {
    let result = run_interpreter(source);
    assert!(
        result.success,
        "Interpreter should succeed.\nSource:\n{}\n\nError: {}",
        source, result.error
    );
    assert_eq!(result.output.trim(), expected, "\nSource:\n{}", source);
}

/// DIFFERENTIAL-CORRECTNESS GATE: the compiled binary and the interpreter MUST
/// agree on a program's outcome (both succeed with identical output, or both
/// fail). This is the CI invariant that catches the whole class of bug where a
/// codegen optimization silently changes a program's meaning relative to the
/// reference engine — e.g. an unsound buffer swap, or an aliased SetIndex
/// double-borrow. The interpreter (VM + tree-walker, cross-checked by the debug
/// shadow oracle) is the reference; codegen must match it.
#[allow(dead_code)]
pub fn assert_compiled_equals_interpreted(source: &str) {
    let compiled = run_logos(source);
    let interp = run_interpreter(source);
    assert_eq!(
        compiled.success, interp.success,
        "DIFFERENTIAL MISMATCH: compiled.success={} but interpreter.success={}\n\
         Source:\n{}\n\ncompiled stderr:\n{}\n\ninterpreter error:\n{}\n\nGenerated Rust:\n{}",
        compiled.success, interp.success, source, compiled.stderr, interp.error, compiled.rust_code
    );
    if compiled.success {
        assert_eq!(
            compiled.stdout.trim(),
            interp.output.trim(),
            "DIFFERENTIAL MISMATCH: compiled and interpreter produced different output\n\
             Source:\n{}\n\nGenerated Rust:\n{}",
            source,
            compiled.rust_code
        );
    }
}

/// The execution tiers a sequential behavioral test runs on by default. EXHAUSTIVE:
/// the `match` in [`run_all_tiers`] has no wildcard arm, so adding a new tier FAILS THE
/// BUILD until it is wired into the default differential — no execution tier can ship
/// without joining Invariant II. (The WASM/AOT tiers are held to the same byte-identity
/// contract by `wasm_aot_lock.rs` and the tier-parity census over their own corpora; this
/// in-process helper covers the tiers every build has, so every ordinary behavioral test
/// gets cross-tier coverage for free.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Tree-walking interpreter — the reference oracle.
    TreeWalker,
    /// Register bytecode VM, on the program as resolved.
    Vm,
    /// The path `largo run --interpret` takes: the optimizer at its full tier, then the VM.
    OptimizedVm,
    /// The optimized VM with the native tier compiling the program's hot loops and functions.
    OptimizedVmNative,
}

/// The single agreed outcome after a program has run identically on every tier.
pub struct TierAgreement {
    pub output: String,
    pub error: Option<String>,
}

/// Run `source` on EVERY applicable execution tier and assert they agree byte-for-byte
/// (output AND error text), returning the agreed outcome. This is the differential-by-
/// default path for SEQUENTIAL behavioral tests (Invariant II): a feature added with
/// only single-tier coverage is a gap, so prefer [`assert_all_tiers`] over the
/// single-tier asserts. The tree-walker is the oracle; every other tier must match it —
/// the VM, the optimized VM, and the optimized VM with the native tier: neither
/// optimizing a program nor compiling part of it to machine code changes what it prints
/// or the sentence it stops with.
///
/// Concurrency programs are out of scope here (the plain VM entry errors on scheduler
/// ops); those have their own seeded differential (`concurrency_differential.rs`).
///
/// Output is compared modulo surrounding whitespace (`.trim()`) — the one benign,
/// well-known tier formatting difference is a trailing newline (the tree-walker joins
/// lines, the VM adds a terminator). This matches the established cross-engine contract
/// used by `assert_compiled_equals_interpreted`. Error text is compared EXACTLY.
#[allow(dead_code)]
pub fn run_all_tiers(source: &str) -> TierAgreement {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_tiered};
    use logicaffeine_compile::optimization::Tier as OptimizerTier;

    let tw = tw_outcome(source);
    #[cfg(not(target_arch = "wasm32"))]
    let forge = logicaffeine_jit::ForgeTier::new();
    for tier in [Tier::TreeWalker, Tier::Vm, Tier::OptimizedVm, Tier::OptimizedVmNative] {
        // No wildcard arm — a new Tier variant makes this non-exhaustive and fails the build.
        let run = match tier {
            Tier::TreeWalker => continue, // the oracle compares against itself
            Tier::Vm => vm_outcome(source),
            Tier::OptimizedVm => vm_outcome_tiered(source, &[], OptimizerTier::T3, None),
            Tier::OptimizedVmNative => {
                #[cfg(not(target_arch = "wasm32"))]
                let native: Option<&dyn logicaffeine_compile::vm::NativeTier> = Some(&forge);
                #[cfg(target_arch = "wasm32")]
                let native: Option<&dyn logicaffeine_compile::vm::NativeTier> = None;
                vm_outcome_tiered(source, &[], OptimizerTier::T3, native)
            }
        };
        assert_eq!(
            (run.output.trim(), run.error.as_deref()),
            (tw.output.trim(), tw.error.as_deref()),
            "TIER DIVERGENCE — Invariant II violated: {tier:?} disagrees with the \
             tree-walker.\nSource:\n{source}\n\ntree-walker: output={:?} error={:?}\n\
             {tier:?}: output={:?} error={:?}",
            tw.output,
            tw.error,
            run.output,
            run.error
        );
    }
    TierAgreement {
        output: tw.output,
        error: tw.error,
    }
}

/// Assert `source` runs cleanly, produces exactly `expected` (trimmed), AND agrees
/// across every execution tier. The drop-in cross-tier replacement for the
/// single-tier `assert_interpreter_output` — behavioral tests should reach for this.
#[allow(dead_code)]
pub fn assert_all_tiers(source: &str, expected: &str) {
    let agreed = run_all_tiers(source);
    assert!(
        agreed.error.is_none(),
        "Program should run cleanly on all tiers.\nSource:\n{source}\n\nError: {}",
        agreed.error.as_deref().unwrap_or("")
    );
    assert_eq!(agreed.output.trim(), expected, "\nSource:\n{source}");
}

/// [`run_all_tiers`] under a deadline: a program that has not finished on every tier within
/// `limit` fails the test instead of hanging it. For programs whose claim is that they finish — a
/// supertask resolved in finite time — finishing is part of what the test asserts.
#[allow(dead_code)]
pub fn run_all_tiers_within(source: &str, limit: std::time::Duration) -> TierAgreement {
    let program = source.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(std::panic::catch_unwind(|| run_all_tiers(&program)));
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(agreed)) => agreed,
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(_) => panic!("the program did not finish on every tier within {limit:?}.\nSource:\n{source}"),
    }
}

/// [`assert_all_tiers`] under a deadline; see [`run_all_tiers_within`].
#[allow(dead_code)]
pub fn assert_all_tiers_within(source: &str, expected: &str, limit: std::time::Duration) {
    let agreed = run_all_tiers_within(source, limit);
    assert!(
        agreed.error.is_none(),
        "Program should run cleanly on all tiers.\nSource:\n{source}\n\nError: {}",
        agreed.error.as_deref().unwrap_or("")
    );
    assert_eq!(agreed.output.trim(), expected, "\nSource:\n{source}");
}

/// Compile a program in **Mode B (deterministic replay)** and run it under
/// `LOGOS_SEED=<seed>`, so a `Select` over multiple ready arms reproduces the
/// interpreter's seeded winner. Mirrors [`run_logos`] but with the deterministic
/// compile + the seed in the environment.
#[allow(dead_code)]
pub fn run_logos_seeded(source: &str, seed: u64) -> E2EResult {
    let compile_output = match compile_program_full_deterministic(source) {
        Ok(out) => out,
        Err(e) => {
            return E2EResult {
                stdout: String::new(),
                stderr: format!("LOGOS compile error: {:?}", e),
                success: false,
                rust_code: String::new(),
            };
        }
    };
    let rust_code = compile_output.rust_code;
    let user_deps = format_user_deps(&compile_output.dependencies);

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let project_dir = temp_dir.path();
    let pkg_id = COMPILE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pkg_name = format!("logos_seeded_{}_{}", get_run_id(), pkg_id);
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "{}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
memchr = "2"
{}"#,
        pkg_name,
        workspace_root.display(),
        workspace_root.display(),
        user_deps
    );

    std::fs::create_dir_all(project_dir.join("src")).unwrap();
    std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(project_dir.join("src/main.rs"), &rust_code).unwrap();
    seed_lockfile(workspace_root, project_dir);

    let run = |offline: bool| {
        let mut args = vec!["run", "--quiet"];
        if offline {
            args.push("--offline");
        }
        Command::new("cargo")
            .args(&args)
            .current_dir(project_dir)
            .env("CARGO_TARGET_DIR", get_shared_target_dir())
            .env("RUST_MIN_STACK", "268435456")
            .env("LOGOS_SEED", seed.to_string())
            .output()
            .expect("cargo run")
    };
    let output = run(true);
    let output = if !output.status.success()
        && String::from_utf8_lossy(&output.stderr).contains("--offline")
    {
        run(false)
    } else {
        output
    };

    prune_project_artifacts(get_shared_target_dir(), &pkg_name);

    E2EResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        success: output.status.success(),
        rust_code,
    }
}

/// SEEDED DIFFERENTIAL GATE: the Mode-B compiled binary at `seed` must produce
/// byte-identical output to the interpreter's seeded run at the same seed. This
/// is the nondeterminate-fragment analogue of [`assert_compiled_equals_interpreted`]:
/// where the determinate fragment needs no seed (Kahn), the nondeterminate
/// fragment is pinned by sharing the choice function.
#[allow(dead_code)]
pub fn assert_compiled_equals_interpreted_seeded(source: &str, seed: u64) {
    use logicaffeine_compile::run_treewalker_concurrent_seeded;

    let compiled = run_logos_seeded(source, seed);
    let interp = run_treewalker_concurrent_seeded(source, seed);
    let interp_ok = interp.error.is_none();
    assert_eq!(
        compiled.success, interp_ok,
        "SEEDED DIFFERENTIAL MISMATCH (seed {seed}): compiled.success={} interpreter.ok={}\n\
         Source:\n{}\n\ncompiled stderr:\n{}\n\ninterpreter error:\n{:?}\n\nGenerated Rust:\n{}",
        compiled.success, interp_ok, source, compiled.stderr, interp.error, compiled.rust_code
    );
    if compiled.success {
        assert_eq!(
            compiled.stdout.trim(),
            interp.lines.join("\n").trim(),
            "SEEDED DIFFERENTIAL MISMATCH (seed {seed}): compiled vs interpreter output differ\n\
             Source:\n{}\n\nGenerated Rust:\n{}",
            source,
            compiled.rust_code
        );
    }
}

/// EQUIVALENCE PROOF, strong form: two spellings of the same construct must
/// lower to BYTE-IDENTICAL generated Rust. This is the gate every
/// parser-tier desugaring ships with — if the old and new spellings build the
/// same AST, the emitted program is the same string, which proves the sugar
/// added zero new runtime surface.
#[allow(dead_code)]
pub fn assert_identical_lowering(old_src: &str, new_src: &str) {
    let old = compile_program_full(old_src).unwrap_or_else(|e| {
        panic!(
            "old spelling failed to compile: {:?}\nSource:\n{}",
            e, old_src
        )
    });
    let new = compile_program_full(new_src).unwrap_or_else(|e| {
        panic!(
            "new spelling failed to compile: {:?}\nSource:\n{}",
            e, new_src
        )
    });
    assert_eq!(
        old.rust_code, new.rust_code,
        "LOWERING MISMATCH: the two spellings did not produce identical Rust\n\
         Old source:\n{}\n\nNew source:\n{}",
        old_src, new_src
    );
}

/// EQUIVALENCE PROOF, behavioral form: for sugar that cannot promise identical
/// lowering (a new builtin or runtime node), both spellings must produce the
/// same interpreter outcome (tree-walker + VM, cross-checked by the shadow
/// oracle), and the new spelling must hold the compiled-vs-interpreted
/// differential in its own right.
#[allow(dead_code)]
pub fn assert_same_meaning(old_src: &str, new_src: &str) {
    let old = run_interpreter(old_src);
    let new = run_interpreter(new_src);
    assert_eq!(
        old.success, new.success,
        "MEANING MISMATCH: old.success={} but new.success={}\n\
         Old source:\n{}\n\nNew source:\n{}\n\nold error: {}\nnew error: {}",
        old.success, new.success, old_src, new_src, old.error, new.error
    );
    assert_eq!(
        old.output.trim(),
        new.output.trim(),
        "MEANING MISMATCH: the two spellings produced different output\n\
         Old source:\n{}\n\nNew source:\n{}",
        old_src,
        new_src
    );
    assert_compiled_equals_interpreted(new_src);
}

/// Differential gate + an expected value, so a regression in BOTH engines (they
/// agree but on the wrong answer) is still caught.
#[allow(dead_code)]
pub fn assert_compiled_equals_interpreted_eq(source: &str, expected: &str) {
    assert_compiled_equals_interpreted(source);
    let interp = run_interpreter(source);
    assert_eq!(
        interp.output.trim(),
        expected,
        "Both engines agree but not on the expected value.\nSource:\n{}",
        source
    );
}

/// Assert that LOGOS code produces exactly the expected lines via the interpreter.
#[allow(dead_code)]
pub fn assert_interpreter_output_lines(source: &str, expected_lines: &[&str]) {
    let result = run_interpreter(source);
    assert!(
        result.success,
        "Interpreter should succeed.\nSource:\n{}\n\nError: {}",
        source, result.error
    );
    let actual_lines: Vec<&str> = result.output.trim().lines().collect();
    assert_eq!(
        actual_lines.len(),
        expected_lines.len(),
        "Line count mismatch.\nExpected {} lines: {:?}\nGot {} lines: {:?}\n\nSource:\n{}",
        expected_lines.len(),
        expected_lines,
        actual_lines.len(),
        actual_lines,
        source
    );
    for (i, (actual, expected)) in actual_lines.iter().zip(expected_lines.iter()).enumerate() {
        assert_eq!(
            actual.trim(),
            *expected,
            "Line {} mismatch.\nExpected: '{}'\nGot:      '{}'\n\nFull output:\n{}\n\nSource:\n{}",
            i + 1,
            expected,
            actual.trim(),
            result.output.trim(),
            source
        );
    }
}

/// Assert that LOGOS code runs successfully via the interpreter (no output check).
#[allow(dead_code)]
pub fn assert_interpreter_runs(source: &str) {
    let result = run_interpreter(source);
    assert!(
        result.success,
        "Interpreter should succeed.\nSource:\n{}\n\nError: {}",
        source, result.error
    );
}

/// Assert that LOGOS source fails to compile (parse error, analysis error, etc.)
/// and that the error message contains the expected substring.
#[allow(dead_code)]
pub fn assert_compile_fails(source: &str, expected_error: &str) {
    let result = run_logos(source);
    assert!(
        !result.success,
        "Expected compilation to fail, but it succeeded.\nSource:\n{}\n\nstdout: {}",
        source, result.stdout
    );
    let all_errors = format!("{}\n{}", result.stderr, result.stdout);
    assert!(
        all_errors.contains(expected_error),
        "Expected error containing '{}' but got:\n{}\n\nSource:\n{}",
        expected_error,
        all_errors,
        source
    );
}

/// Assert that LOGOS source fails during interpretation
/// and that the error message contains the expected substring.
#[allow(dead_code)]
pub fn assert_interpreter_fails(source: &str, expected_error: &str) {
    let result = run_interpreter(source);
    assert!(
        !result.success,
        "Expected interpreter to fail, but it succeeded.\nSource:\n{}\n\nOutput: {}",
        source, result.output
    );
    assert!(
        result.error.contains(expected_error),
        "Expected error containing '{}' but got:\n{}\n\nSource:\n{}",
        expected_error,
        result.error,
        source
    );
}

/// Assert that LOGOS code produces output containing the expected substring via the interpreter.
#[allow(dead_code)]
pub fn assert_interpreter_output_contains(source: &str, expected: &str) {
    let result = run_interpreter(source);
    assert!(
        result.success,
        "Interpreter should succeed.\nSource:\n{}\n\nError: {}",
        source, result.error
    );
    assert!(
        result.output.trim().contains(expected),
        "Expected '{}' in output.\nGot: '{}'\n\nSource:\n{}",
        expected,
        result.output.trim(),
        source
    );
}
