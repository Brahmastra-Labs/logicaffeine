//! Model bring-up: a checkpoint's one program, lowered by every chip that runs it.
//!
//! A model is written once: [`crate::model::author`] writes its computation as a LOGOS program from the checkpoint's
//! own spec, and [`crate::model::program`] names each cut of it. A chip is a [`Backend`]: it lowers the cuts it runs,
//! writes its artifacts, and names the cut each artifact was lowered from — or refuses the model by name. Adding a
//! model takes no backend code; adding a chip is one more [`Backend`] in [`backends`].
//!
//! [`bring_up`] runs a checkpoint through the backends asked for and reports, for each, the programs it lowered or why
//! it refused, beside the identity of the decode step every one of them is a cut of. [`run_sequences`] then runs the
//! model on every backend that brought it up and is on this machine, every [`Sequence`] it is given — a CPU tier as one
//! generation program built once: the decode step's own statements in a loop over positions, the weights read once,
//! each sequence's state declared afresh — each sequence judged by how many of its tokens lead its reference's (the
//! model's f64 reference, or a golden's continuation). A chip that is not here, or whose run is not built yet, says
//! which.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::codegen_cuda::knobs::CudaKnobs;
use crate::codegen_cuda::lane::{compile, LaneOptions};
use crate::model::bind::{bind, rotary_values, values, Source};
use crate::model::gguf::Gguf;
use crate::model::program::{written, Cut, Identity};
use crate::model::quant::GgmlType;
use crate::model::spec::ModelSpec;
use crate::ui_bridge::{interpret_for_ui_sync, set_engine, vm_listing, Engine};

/// What a backend made of a model: the cut of the one program behind every artifact, and the files it wrote.
#[derive(Clone, Debug)]
pub struct Brought {
    pub programs: Vec<Identity>,
    pub files: Vec<PathBuf>,
}

/// Why a backend does not run a model, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A chip a model's one program is lowered to.
pub trait Backend {
    /// The chip, as the command line names it.
    fn name(&self) -> &'static str;
    /// Lower `spec`'s program — the checkpoint at `checkpoint` — over caches of `ctx` positions, writing the artifacts
    /// under `out`.
    fn bring_up(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused>;
    /// Run the model on this chip, what [`Backend::bring_up`] wrote under `out` beside it: every one of `sequences`
    /// from an empty state, its prompt (token ids from 0) and then as many tokens as it is judged against, generated
    /// greedily through the one program — or refuse by name, a chip that is not on this machine, or whose run is not
    /// built yet, saying which.
    fn run(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused>;
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Refused + '_ {
    move |e| Refused(format!("{}: {e}", path.display()))
}

fn write(path: PathBuf, bytes: impl AsRef<[u8]>, files: &mut Vec<PathBuf>) -> Result<(), Refused> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    std::fs::write(&path, bytes).map_err(io(&path))?;
    files.push(path);
    Ok(())
}

/// One sequence a run generates: its prompt (token ids from 0), and the tokens it is judged against — as many as it
/// generates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence {
    pub prompt: Vec<u32>,
    pub tokens: Vec<u32>,
}

/// What a chip generated for a run's sequences: their tokens, a list a sequence in their order (ids from 0); the
/// seconds the runs took, not the building of what ran them; what ran them — the chip itself, or the model of it this
/// machine holds; and the program it ran — the generation program a CPU tier built, or the artifact the chip runs.
#[derive(Clone, Debug)]
pub struct Generated {
    pub tokens: Vec<Vec<u32>>,
    pub seconds: f64,
    pub on: String,
    pub program: PathBuf,
}

impl Generated {
    /// Generated tokens a second over every sequence, each prompt's inside the time.
    pub fn tok_s(&self) -> f64 {
        self.tokens.iter().map(Vec::len).sum::<usize>() as f64 / self.seconds.max(f64::MIN_POSITIVE)
    }
}

/// A backend's run of one sequence: the tokens it generated after the prompt (ids from 0), the seconds from the prompt
/// to the last token, and what ran them — the chip itself, or the model of it this machine holds.
#[derive(Clone, Debug)]
pub struct Ran {
    pub tokens: Vec<u32>,
    pub seconds: f64,
    pub on: String,
}

impl Ran {
    /// Generated tokens a second, the prompt's prefill inside the time.
    pub fn tok_s(&self) -> f64 {
        self.tokens.len() as f64 / self.seconds.max(f64::MIN_POSITIVE)
    }
}

/// How many of `ran`'s tokens agree with `reference`'s before the first that does not.
pub fn agreeing(ran: &[u32], reference: &[u32]) -> usize {
    ran.iter().zip(reference).take_while(|(a, b)| a == b).count()
}

/// What a run is judged against: the tokens it should generate, and where they come from.
#[derive(Clone, Debug)]
pub struct Reference {
    pub tokens: Vec<u32>,
    pub what: String,
}

impl Reference {
    /// The model's f64 reference ([`crate::model::reference`]) generating `tokens` tokens greedily after `prompt`,
    /// reading the weights of the checkpoint at `checkpoint` — a GGUF file or a Hugging Face directory.
    pub fn of(spec: &ModelSpec, checkpoint: &Path, prompt: &[u32], tokens: usize) -> Result<Reference, Refused> {
        let g = Opened(crate::model::open_checkpoint(checkpoint).map_err(io(checkpoint))?);
        let refused = |e: crate::model::reference::ReferenceError| Refused(format!("the f64 reference: {e}"));
        let mut r = crate::model::reference::Reference::new(spec.clone(), g).map_err(refused)?;
        let mut logits = r.step(prompt).map_err(refused)?;
        let mut generated = Vec::with_capacity(tokens);
        while generated.len() < tokens {
            let next = crate::model::reference::argmax(&logits);
            generated.push(next);
            if generated.len() < tokens {
                logits = r.step(&[next]).map_err(refused)?;
            }
        }
        Ok(Reference { tokens: generated, what: "the f64 reference".to_string() })
    }

    /// A golden's first prompt as a run's prompt, and its continuation as the tokens the run is judged against
    /// ([`Reference::golden_sequences`]).
    pub fn golden(path: &Path) -> Result<(Vec<u32>, Reference), Refused> {
        let (sequences, what) = Reference::golden_sequences(path)?;
        let first = sequences.into_iter().next().ok_or_else(|| Refused(format!("{}: no prompt", path.display())))?;
        Ok((first.prompt, Reference { tokens: first.tokens, what }))
    }

    /// Every prompt of a golden (`model_golden_lock`'s format: each of `prompts` with its `tokens` and its
    /// `continuation`, ids from 0) as a sequence judged against its own continuation, and the golden, named.
    pub fn golden_sequences(path: &Path) -> Result<(Vec<Sequence>, String), Refused> {
        let text = std::fs::read_to_string(path).map_err(io(path))?;
        let g: serde_json::Value = serde_json::from_str(&text).map_err(|e| Refused(format!("{}: {e}", path.display())))?;
        let prompts = g["prompts"].as_array().filter(|p| !p.is_empty()).ok_or_else(|| Refused(format!("{}: no prompt", path.display())))?;
        let ids = |i: usize, key: &str| -> Result<Vec<u32>, Refused> {
            prompts[i][key]
                .as_array()
                .ok_or_else(|| Refused(format!("{}: prompt {} has no `{key}`", path.display(), i + 1)))?
                .iter()
                .map(|t| t.as_u64().map(|t| t as u32).ok_or_else(|| Refused(format!("{}: a `{key}` entry is not a token", path.display()))))
                .collect()
        };
        let sequences = (0..prompts.len()).map(|i| Ok(Sequence { prompt: ids(i, "tokens")?, tokens: ids(i, "continuation")? })).collect::<Result<_, Refused>>()?;
        Ok((sequences, format!("the golden {}", path.display())))
    }
}

/// A checkpoint opened as either kind — a GGUF file or a Hugging Face directory — handed to what reads one source.
struct Opened(Box<dyn crate::model::Checkpoint>);

impl crate::model::reference::WeightSource for Opened {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        crate::model::reference::WeightSource::describe(self.0.as_ref(), name)
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        crate::model::reference::WeightSource::row_bytes(self.0.as_mut(), name, first, count)
    }
    fn has(&self, name: &str) -> bool {
        crate::model::reference::WeightSource::has(self.0.as_ref(), name)
    }
}

/// Whether `prompt` and `tokens` more fit caches of `ctx` positions: the prompt fills positions 1 to its length, and
/// every generated token but the last is fed back at the next.
fn fits(prompt: &[u32], tokens: usize, ctx: usize) -> Result<(), Refused> {
    if prompt.is_empty() || prompt.len() + tokens > ctx + 1 {
        return Err(Refused(format!("a prompt of {} tokens and {tokens} more do not fit {ctx} positions", prompt.len())));
    }
    Ok(())
}

/// The length a declaration's shape `R by C tensor of Real.` gives.
fn declared_len(shape: &str) -> Option<usize> {
    let (r, rest) = shape.split_once(" by ")?;
    Some(r.trim().parse::<usize>().ok()? * rest.split_whitespace().next()?.parse::<usize>().ok()?)
}

/// `values` written to `path` as the file `readReals` reads: little-endian binary64, eight bytes a value in order.
fn write_reals(path: &Path, values: &[f64]) -> Result<(), Refused> {
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(path, bytes).map_err(io(path))
}

/// The names a generation program wraps the decode step in; a step that names one of them is refused.
const RUN_NAMES: [&str; 9] =
    ["runTokens", "runStarts", "runLengths", "runCounts", "runSequence", "runStart", "runLength", "runCount", "runLast"];

/// The names the decode step binds its two inputs to, the token and the position: the lines where the step at token 2,
/// position 3 differs from the step at its placeholders, each `Let <name> be 1.` there.
fn inputs(spec: &ModelSpec, ctx: usize, text: &str) -> Result<(String, String), Refused> {
    let (other, _, _) = crate::model::program::decode_at(spec, "", ctx, 2, 3);
    let (a, b): (Vec<&str>, Vec<&str>) = (text.lines().collect(), other.lines().collect());
    if a.len() != b.len() {
        return Err(Refused("the decode step's inputs change its lines".to_string()));
    }
    let (mut token, mut position) = (None, None);
    for (x, y) in a.iter().zip(&b).filter(|(x, y)| x != y) {
        let var = x.strip_prefix("Let ").and_then(|r| r.strip_suffix(" be 1.")).ok_or_else(|| Refused(format!("`{x}`: an input the step does not bind")))?;
        match y.strip_prefix(&format!("Let {var} be ")) {
            Some("2.") if token.is_none() => token = Some(var.to_string()),
            Some("3.") if position.is_none() => position = Some(var.to_string()),
            _ => return Err(Refused(format!("`{x}`: an input the step does not bind"))),
        }
    }
    match (token, position) {
        (Some(t), Some(p)) => Ok((t, p)),
        _ => Err(Refused("the decode step binds no token or no position".to_string())),
    }
}

/// The decode step as one generation program for `sequences`, written to `dir/generation.lg` with every weight's
/// values beside it in the file `readReals` reads. Every weight is declared and read once, before the loops; each
/// sequence declares the state its step carries afresh (the caches, memories and rings it binds as state), so none
/// sees another's; and the step's own statements — every line of the decode step but the bindings of its token and
/// position, in order — run in a loop over its positions, the prompt fed a token a step and each token after it the
/// step's own `next`, shown from the prompt's last position on. A tensor of the step's own working stays in the loop,
/// declared as the step declares it. Returns the program's path and text.
fn generation(spec: &ModelSpec, checkpoint: &Path, ctx: usize, sequences: &[Sequence], dir: &Path) -> Result<(PathBuf, String), Refused> {
    if sequences.is_empty() {
        return Err(Refused("a run of no sequence".to_string()));
    }
    for s in sequences {
        fits(&s.prompt, s.tokens.len(), ctx)?;
    }
    let ((text, f32w, q8w), _) = written(spec, Cut::Decode { ctx });
    let (token, position) = inputs(spec, ctx, &text)?;
    if let Some(name) = RUN_NAMES.iter().find(|n| text.contains(*n)) {
        return Err(Refused(format!("the decode step names `{name}`, a name its generation program binds")));
    }
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let mut opened = crate::model::open_checkpoint(checkpoint).map_err(io(checkpoint))?;
    let source: &mut dyn crate::model::reference::WeightSource = opened.as_mut();
    let (bound_token, bound_position) = (format!("Let {token} be 1."), format!("Let {position} be 1."));
    let (mut weights, mut state, mut body) = (String::new(), String::new(), String::new());
    for line in text.lines() {
        if line == "## Main" || line.trim().is_empty() || line == bound_token || line == bound_position {
            continue;
        }
        if let Some((var, shape)) = line.strip_prefix("Let ").and_then(|r| r.split_once(" be a ")) {
            let named = f32w.contains_key(var) || q8w.contains_key(var);
            let size = || declared_len(shape).ok_or_else(|| Refused(format!("`{line}`: a declaration a run cannot size")));
            match bind(spec, var) {
                Ok(Source::State) => {
                    state.push_str(&format!("    {line}\n"));
                    continue;
                }
                bound if named => {
                    let v = match (rotary_values(spec, var, ctx, source).map_err(|e| Refused(format!("`{var}`: {e}")))?, bound) {
                        (Some(v), _) => v,
                        (None, Ok(Source::Weight(b))) => values(&b, source).map_err(|e| Refused(format!("`{var}`: {e}")))?,
                        (None, _) => return Err(Refused(format!("`{var}`: the manifest names it and nothing binds it"))),
                    };
                    let len = size()?;
                    if v.len() != len {
                        return Err(Refused(format!("`{var}` declares {len} values and its source holds {}", v.len())));
                    }
                    let path = dir.join(format!("{var}.f64"));
                    write_reals(&path, &v)?;
                    weights.push_str(&format!("{line}\nSet {var} to readReals(\"{}\").\n", path.display()));
                    continue;
                }
                Ok(Source::Weight(_)) => return Err(Refused(format!("`{var}` binds a weight the program's manifest does not name"))),
                Err(_) => {}
            }
        }
        body.push_str(&format!("        {line}\n"));
    }
    let list = |xs: Vec<usize>| xs.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
    let starts = sequences.iter().scan(1, |at, s| {
        let start = *at;
        *at += s.prompt.len();
        Some(start)
    });
    let mut out = String::from("## Main\n");
    out.push_str(&weights);
    out.push_str(&format!("Let runTokens be [{}].\n", list(sequences.iter().flat_map(|s| s.prompt.iter().map(|&t| t as usize + 1)).collect())));
    out.push_str(&format!("Let runStarts be [{}].\n", list(starts.collect())));
    out.push_str(&format!("Let runLengths be [{}].\n", list(sequences.iter().map(|s| s.prompt.len()).collect())));
    out.push_str(&format!("Let runCounts be [{}].\n", list(sequences.iter().map(|s| s.tokens.len()).collect())));
    out.push_str(&format!("Repeat for runSequence from 1 to {}:\n", sequences.len()));
    out.push_str(&state);
    out.push_str("    Let runStart be item runSequence of runStarts.\n");
    out.push_str("    Let runLength be item runSequence of runLengths.\n");
    out.push_str("    Let runCount be item runSequence of runCounts.\n");
    out.push_str("    Let runLast be runLength + runCount - 1.\n");
    out.push_str(&format!("    Let mutable {token} be item runStart of runTokens.\n"));
    out.push_str(&format!("    Repeat for {position} from 1 to runLast:\n"));
    out.push_str(&body);
    out.push_str(&format!("        If {position} is at least runLength:\n            Show next.\n"));
    out.push_str(&format!("        If {position} is less than runLength:\n            Set {token} to item (runStart + {position}) of runTokens.\n"));
    out.push_str(&format!("        Otherwise:\n            Set {token} to next.\n"));
    let path = dir.join("generation.lg");
    std::fs::write(&path, &out).map_err(io(&path))?;
    Ok((path, out))
}

/// A generation program's shown tokens read back: every sequence's, in order, from 1 in the program and from 0 here.
fn read_generated(lines: &[String], sequences: &[Sequence]) -> Result<Vec<Vec<u32>>, Refused> {
    let want: usize = sequences.iter().map(|s| s.tokens.len()).sum();
    if lines.len() != want {
        return Err(Refused(format!("the run showed {} tokens, {want} expected", lines.len())));
    }
    let mut shown = lines.iter();
    sequences
        .iter()
        .map(|s| {
            (0..s.tokens.len())
                .map(|_| {
                    let line = shown.next().map(|l| l.trim()).unwrap_or_default();
                    let t: u32 = line.parse().map_err(|_| Refused(format!("the shown token `{line}` is not a token")))?;
                    t.checked_sub(1).ok_or_else(|| Refused("the run's next token is 0, below the vocabulary".to_string()))
                })
                .collect()
        })
        .collect()
}

/// Generate every sequence on a CPU tier: the generation program written under `dir`, then built and run once by
/// `go` (its text in, the lines it showed and the seconds its run took out).
fn generate(
    spec: &ModelSpec,
    checkpoint: &Path,
    ctx: usize,
    sequences: &[Sequence],
    on: &str,
    dir: &Path,
    go: impl FnOnce(&str) -> Result<(Vec<String>, f64), Refused>,
) -> Result<Generated, Refused> {
    let (program, text) = generation(spec, checkpoint, ctx, sequences, dir)?;
    let (lines, seconds) = go(&text)?;
    Ok(Generated { tokens: read_generated(&lines, sequences)?, seconds, on: on.to_string(), program })
}

/// One cut run in this process on `engine`: the lines it showed and the seconds the run took.
fn in_process(engine: Engine, text: &str) -> Result<(Vec<String>, f64), Refused> {
    crate::semantics::storage::reset();
    set_engine(engine);
    let start = Instant::now();
    let result = interpret_for_ui_sync(text);
    let seconds = start.elapsed().as_secs_f64();
    set_engine(Engine::Auto);
    match result.error {
        Some(e) => Err(Refused(e)),
        None => Ok((result.lines, seconds)),
    }
}

/// Run a built program: the lines it wrote and the seconds it ran, or how it failed.
fn timed(mut cmd: Command, what: &str) -> Result<(Vec<String>, f64), Refused> {
    let start = Instant::now();
    let o = cmd.output().map_err(|e| Refused(format!("{what}: {e}")))?;
    let seconds = start.elapsed().as_secs_f64();
    if !o.status.success() {
        return Err(Refused(format!("{what} exited {:?}: {}", o.status.code(), String::from_utf8_lossy(&o.stderr).trim())));
    }
    Ok((String::from_utf8_lossy(&o.stdout).lines().map(str::to_string).collect(), seconds))
}

/// Build with `cmd`, or refuse with the tool's own report.
fn built(mut cmd: Command, what: &str) -> Result<(), Refused> {
    let o = cmd.output().map_err(|e| Refused(format!("{what}: {e}")))?;
    if !o.status.success() {
        return Err(Refused(format!("{what} failed: {}", String::from_utf8_lossy(&o.stderr).trim())));
    }
    Ok(())
}

/// Whether `tool` runs on this machine (`tool --version` succeeds).
fn on_path(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok_and(|o| o.status.success())
}

/// A tier's compiler refusing a program, in the compiler's words.
fn not_compiled(tier: &str, e: crate::ParseError) -> Refused {
    Refused(format!("the {tier} backend does not compile the program: {:?}", e.kind))
}

/// The decode step's text, lowered by `lower` into `out/<file>` (each `(file, contents)` it returns): the artifacts a
/// CPU tier's bring-up writes, every one of them the decode step's.
fn lowered(spec: &ModelSpec, ctx: usize, out: &Path, lower: impl FnOnce(&str) -> Result<Vec<(&'static str, Vec<u8>)>, Refused>) -> Result<Brought, Refused> {
    let ((text, _, _), decode) = written(spec, Cut::Decode { ctx });
    let mut files = Vec::new();
    for (file, bytes) in lower(&text)? {
        write(out.join(file), bytes, &mut files)?;
    }
    Ok(Brought { programs: vec![decode], files })
}

/// The bytecode VM ([`crate::vm`]): the decode step compiled to the bytecode it runs, as `largo run --interpret`
/// compiles it ([`vm_listing`]).
pub struct Vm;

impl Backend for Vm {
    fn name(&self) -> &'static str {
        "vm"
    }

    fn bring_up(&self, spec: &ModelSpec, _checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        lowered(spec, ctx, out, |text| Ok(vec![("decode.bytecode", vm_listing(text).map_err(Refused)?.into_bytes())]))
    }

    fn run(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused> {
        generate(spec, checkpoint, ctx, sequences, "the bytecode VM", &out.join("run"), |text| in_process(Engine::Vm, text))
    }
}

/// The VM with its native tier: the same bytecode, the loops it runs hot compiled to machine code as it runs.
pub struct Jit;

impl Backend for Jit {
    fn name(&self) -> &'static str {
        "jit"
    }

    fn bring_up(&self, spec: &ModelSpec, _checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        lowered(spec, ctx, out, |text| Ok(vec![("decode.bytecode", vm_listing(text).map_err(Refused)?.into_bytes())]))
    }

    fn run(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused> {
        generate(spec, checkpoint, ctx, sequences, "the VM with its JIT", &out.join("run"), |text| in_process(Engine::Jit, text))
    }
}

/// The C compiler a run builds with: `$CC`, else `cc`.
fn c_compiler() -> String {
    std::env::var("CC").ok().filter(|cc| !cc.trim().is_empty()).unwrap_or_else(|| "cc".to_string())
}

/// The CPU through C ([`crate::compile::compile_to_c`]): the decode step as one C program; a run builds its
/// generation program once, with no contraction of a product and a sum into one fused multiply-add, whose single
/// rounding the VM's arithmetic never takes, and runs it.
pub struct C;

impl Backend for C {
    fn name(&self) -> &'static str {
        "c"
    }

    fn bring_up(&self, spec: &ModelSpec, _checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        lowered(spec, ctx, out, |text| {
            Ok(vec![("decode.c", crate::compile::compile_to_c(text).map_err(|e| not_compiled("C", e))?.into_bytes())])
        })
    }

    fn run(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused> {
        let cc = c_compiler();
        if !on_path(&cc) {
            return Err(Refused(format!("no C compiler on this machine: `{cc}` does not run")));
        }
        let dir = out.join("run");
        generate(spec, checkpoint, ctx, sequences, "the CPU, through C", &dir, |text| {
            let (src, bin) = (dir.join("generation.c"), dir.join("generation"));
            let c = crate::compile::compile_to_c(text).map_err(|e| not_compiled("C", e))?;
            std::fs::write(&src, c).map_err(io(&src))?;
            let mut build = Command::new(&cc);
            build.args(["-O2", "-ffp-contract=off", "-o"]).arg(&bin).arg(&src).arg("-lm");
            built(build, &cc)?;
            timed(Command::new(&bin), "the C program")
        })
    }
}

/// A staged project's manifest cut to what a cut of the one program needs: the runtime's file reading (`persistence`,
/// its weights' reader) and none of the rest of it (the network stack, the scheduler), tokio only when the program
/// names it, and full optimization without link-time optimization — a cut builds in seconds where the whole runtime
/// under link-time optimization takes many minutes. Every other line stays as staged.
fn lean_manifest(staged: &str, rust: &str) -> String {
    let mut out = String::with_capacity(staged.len());
    for line in staged.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("lto =") || trimmed.starts_with("codegen-units =") {
            continue;
        }
        if trimmed.starts_with("tokio =") && !rust.contains("tokio") {
            continue;
        }
        let line = line
            .replace("features = [\"full\", \"io-uring\"]", "features = [\"persistence\"]")
            .replace("features = [\"full\"]", "features = [\"persistence\"]");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The CPU through Rust ([`crate::compile::compile_to_rust`]): the decode step as one Rust program; a run stages one
/// project beside the artifacts ([`crate::compile::compile_to_dir`]) and builds its generation program into it in
/// release, only its `src/main.rs` rewritten when the project is already staged, so the runtime crates build once a
/// project.
pub struct Rust;

impl Backend for Rust {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn bring_up(&self, spec: &ModelSpec, _checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        lowered(spec, ctx, out, |text| {
            Ok(vec![("decode.rs", crate::compile::compile_to_rust(text).map_err(|e| not_compiled("Rust", e))?.into_bytes())])
        })
    }

    fn run(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused> {
        if !on_path("cargo") {
            return Err(Refused("no Rust toolchain on this machine: `cargo` does not run".to_string()));
        }
        let (dir, project) = (out.join("run"), out.join("project"));
        generate(spec, checkpoint, ctx, sequences, "the CPU, through Rust", &dir, |text| {
            let rust = crate::compile::compile_program_full(text).map_err(|e| not_compiled("Rust", e))?.rust_code;
            if project.join("Cargo.toml").is_file() {
                let main = project.join("src").join("main.rs");
                std::fs::write(&main, &rust).map_err(io(&main))?;
            } else {
                crate::compile::compile_to_dir(text, &project).map_err(|e| Refused(format!("the Rust backend: {e}")))?;
                let manifest = project.join("Cargo.toml");
                let staged = std::fs::read_to_string(&manifest).map_err(io(&manifest))?;
                std::fs::write(&manifest, lean_manifest(&staged, &rust)).map_err(io(&manifest))?;
            }
            let target = project.join("target");
            let mut build = Command::new("cargo");
            build.args(["build", "--release", "--quiet"]).current_dir(&project).env("CARGO_TARGET_DIR", &target);
            built(build, "cargo build")?;
            timed(Command::new(target.join("release").join("logos_output")), "the Rust program")
        })
    }
}

/// RISC-V through its AOT tier ([`crate::compile::compile_to_riscv`]): the decode step as RISC-V assembly; a run links
/// its generation program with its runtime ([`crate::codegen_riscv::runtime`]) by `riscv64-linux-gnu-gcc` once and
/// runs it under `qemu-riscv64`.
pub struct RiscvQemu;

impl Backend for RiscvQemu {
    fn name(&self) -> &'static str {
        "riscv-qemu"
    }

    fn bring_up(&self, spec: &ModelSpec, _checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        lowered(spec, ctx, out, |text| {
            Ok(vec![("decode.s", crate::compile::compile_to_riscv(text).map_err(|e| not_compiled("RISC-V", e))?.into_bytes())])
        })
    }

    fn run(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused> {
        for tool in ["riscv64-linux-gnu-gcc", "qemu-riscv64"] {
            if !on_path(tool) {
                return Err(Refused(format!("no RISC-V toolchain on this machine: `{tool}` does not run")));
            }
        }
        let dir = out.join("run");
        generate(spec, checkpoint, ctx, sequences, "RISC-V under qemu", &dir, |text| {
            let asm = crate::compile::compile_to_riscv(text).map_err(|e| not_compiled("RISC-V", e))?;
            let runtime = crate::codegen_riscv::runtime(&asm).map_err(|e| Refused(format!("the RISC-V runtime: {e}")))?;
            let (s, rt, bin) = (dir.join("generation.s"), dir.join("logos_rt.c"), dir.join("generation"));
            std::fs::write(&s, &asm).map_err(io(&s))?;
            std::fs::write(&rt, &runtime.c).map_err(io(&rt))?;
            let mut link = Command::new("riscv64-linux-gnu-gcc");
            link.args(["-O2", "-static", "-o"]).arg(&bin).arg(&s).arg(&rt).args(&runtime.link_args);
            built(link, "riscv64-linux-gnu-gcc")?;
            let mut qemu = Command::new("qemu-riscv64");
            qemu.arg(&bin);
            timed(qemu, "qemu-riscv64")
        })
    }
}

/// WebAssembly ([`crate::compile::compile_to_wasm`]): the decode step as a module with the host shim `largo build
/// --emit wasm` writes beside it ([`crate::web_host::wasm_host_shim`]); a run executes its generation program's module
/// under node.
pub struct Wasm;

impl Backend for Wasm {
    fn name(&self) -> &'static str {
        "wasm"
    }

    fn bring_up(&self, spec: &ModelSpec, _checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        lowered(spec, ctx, out, |text| {
            let module = crate::compile::compile_to_wasm(text).map_err(|e| not_compiled("WebAssembly", e))?;
            Ok(vec![("decode.wasm", module), ("decode.mjs", crate::web_host::wasm_host_shim("decode").into_bytes())])
        })
    }

    fn run(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused> {
        if !on_path("node") {
            return Err(Refused("no WebAssembly engine on this machine: `node` does not run".to_string()));
        }
        let dir = out.join("run");
        generate(spec, checkpoint, ctx, sequences, "WebAssembly under node", &dir, |text| {
            let module = crate::compile::compile_to_wasm(text).map_err(|e| not_compiled("WebAssembly", e))?;
            let (wasm, host) = (dir.join("generation.wasm"), dir.join("generation.mjs"));
            std::fs::write(&wasm, module).map_err(io(&wasm))?;
            std::fs::write(&host, crate::web_host::wasm_host_shim("generation")).map_err(io(&host))?;
            let mut node = Command::new("node");
            node.arg(&host);
            timed(node, "node")
        })
    }
}

/// The CUDA lane ([`crate::codegen_cuda::lane`]): the decode step and its prefill chunks as one fused CUDA program
/// and the sidecar of computed constants it reads.
pub struct Cuda;

impl Backend for Cuda {
    fn name(&self) -> &'static str {
        "cuda"
    }

    fn bring_up(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        let prefill = (1..=16usize).rev().find(|r: &usize| r.is_power_of_two() && *r <= ctx).unwrap_or(1);
        let opts = LaneOptions { ctx, knobs: CudaKnobs::default(), prefill };
        let lane = compile(spec, checkpoint, &opts).map_err(|e| Refused(e.to_string()))?;
        let mut files = Vec::new();
        write(out.join("lane.cu"), &lane.program.cu, &mut files)?;
        write(out.join("lane.side"), &lane.program.sidecar, &mut files)?;
        Ok(Brought { programs: lane.forms.iter().map(|f| f.identity).collect(), files })
    }

    /// The lane built by `nvcc` for this machine's GPU once and run on the checkpoint a sequence a run: its own clocks
    /// time each prompt's prefill and every decode step after its first token.
    fn run(&self, _spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused> {
        for s in sequences {
            fits(&s.prompt, s.tokens.len(), ctx)?;
        }
        let gpu = Command::new("nvidia-smi").args(["--query-gpu=compute_cap", "--format=csv,noheader"]).output();
        let cap = match gpu {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().replace('.', ""),
            _ => String::new(),
        };
        if cap.is_empty() || !on_path("nvcc") {
            return Err(Refused("no CUDA GPU on this machine: the lane needs nvcc and a device nvidia-smi lists".to_string()));
        }
        let bin = out.join("lane");
        let mut nvcc = Command::new("nvcc");
        nvcc.args(["-O3", "-std=c++17", &format!("-arch=sm_{cap}"), "-o"]).arg(&bin).arg(out.join("lane.cu"));
        built(nvcc, "nvcc")?;
        let (mut tokens, mut seconds) = (Vec::with_capacity(sequences.len()), 0.0);
        for s in sequences {
            let n = s.tokens.len();
            if n == 0 {
                tokens.push(Vec::new());
                continue;
            }
            let o = Command::new(&bin)
                .arg(checkpoint)
                .arg(out.join("lane.side"))
                .arg(n.to_string())
                .args(s.prompt.iter().map(u32::to_string))
                .output()
                .map_err(|e| Refused(format!("the lane: {e}")))?;
            let stdout = String::from_utf8_lossy(&o.stdout);
            if !o.status.success() {
                return Err(Refused(format!("the lane exited {:?}: {}", o.status.code(), String::from_utf8_lossy(&o.stderr).trim())));
            }
            let field = |key: &str| stdout.lines().find_map(|l| l.strip_prefix(key)).map(str::trim);
            let generated: Result<Vec<u32>, _> = field("TOKENS:").unwrap_or("").split_whitespace().map(str::parse).collect();
            let generated = generated.map_err(|_| Refused("the lane printed a token that is not a number".to_string()))?;
            let number = |key: &str| field(key).and_then(|v| v.parse::<f64>().ok());
            let prefill = number("PREFILL_MS:").ok_or_else(|| Refused("the lane printed no prefill time".to_string()))?;
            let step = if n > 1 { number("DECODE_MS_PER_TOKEN:").ok_or_else(|| Refused("the lane printed no decode time".to_string()))? } else { 0.0 };
            seconds += (prefill + step * (n - 1) as f64) / 1000.0;
            tokens.push(generated);
        }
        Ok(Generated { tokens, seconds, on: "the GPU".to_string(), program: out.join("lane.cu") })
    }
}

/// The D5005 decode engine ([`crate::spatial::decode_engine`]): the step's program for the engine's sequencer, the
/// checkpoint held to what the engine's units read.
pub struct D5005;

impl Backend for D5005 {
    fn name(&self) -> &'static str {
        "d5005"
    }

    fn bring_up(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        use crate::spatial::decode_engine::capability::{refusals, summarize};
        use crate::spatial::decode_engine::model::EngineNumerics;
        use crate::spatial::decode_engine::program::lower;
        let g = Gguf::open(checkpoint).map_err(io(checkpoint))?;
        let tensors: Vec<(String, GgmlType)> = g
            .tensors()
            .iter()
            .filter_map(|t| GgmlType::from_id(t.ggml_type).ok().map(|ty| (t.name.clone(), ty)))
            .collect();
        let gaps = refusals(spec, &tensors);
        if !gaps.is_empty() {
            return Err(Refused(summarize(&gaps, spec.layers.len()).join("; ")));
        }
        let lowered = lower(spec, EngineNumerics::fastest(), ctx).map_err(|e| Refused(e.to_string()))?;
        let mut files = Vec::new();
        let commands: String = lowered.program.commands.iter().map(|c| format!("{c:?}\n")).collect();
        let words: String = lowered
            .program
            .encode()
            .iter()
            .map(|w| format!("{:016x}{:016x}{:016x}{:016x}\n", w[3], w[2], w[1], w[0]))
            .collect();
        write(out.join("commands.txt"), commands, &mut files)?;
        write(out.join("words.hex"), words, &mut files)?;
        write(out.join("program.lg"), lowered.identity.cut.write(spec, "").0, &mut files)?;
        write(out.join("program.txt"), format!("{}\n", lowered.identity.line()), &mut files)?;
        Ok(Brought { programs: vec![lowered.identity], files })
    }

    /// The card is not on this machine, so the engine's machine model runs the program: the same jobs over the same
    /// memory image the card executes, token for token as the card (`d5005_engine_card_lock`), at this computer's speed
    /// — each sequence on a machine of its own, from the empty state the card starts a sequence in.
    fn run(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> Result<Generated, Refused> {
        use crate::spatial::decode_engine::machine::Machine;
        use crate::spatial::decode_engine::model::EngineNumerics;
        for s in sequences {
            fits(&s.prompt, s.tokens.len(), ctx)?;
        }
        let refused = |e: crate::spatial::decode_engine::model::EngineError| Refused(format!("the engine's machine model: {e:?}"));
        let start = Instant::now();
        let mut tokens = Vec::with_capacity(sequences.len());
        for sequence in sequences {
            let g = Gguf::open(checkpoint).map_err(io(checkpoint))?;
            let mut m = Machine::new(spec.clone(), g, EngineNumerics::fastest(), 1, ctx).map_err(refused)?;
            let s = m.sequence().map_err(refused)?;
            let mut feed = sequence.prompt.clone();
            let mut generated = Vec::with_capacity(sequence.tokens.len());
            while generated.len() < sequence.tokens.len() {
                let logits = m.step(&[(s, feed)]).map_err(refused)?;
                let last = logits.first().and_then(|e| e.last()).ok_or_else(|| Refused("the machine returned no logits".to_string()))?;
                let next = (0..last.len()).fold(0, |best, i| if last[i] > last[best] { i } else { best }) as u32;
                generated.push(next);
                feed = vec![next];
            }
            tokens.push(generated);
        }
        Ok(Generated {
            tokens,
            seconds: start.elapsed().as_secs_f64(),
            on: "the engine's machine model (no card on this machine)".to_string(),
            program: out.join("program.lg"),
        })
    }
}

/// The SambaNova SN50 ([`crate::spatial::rdu_chip`]): the decode deployment as one certified program per layer kind,
/// the embedding and the head, bound layer by layer by a manifest — SDK-gated, its placements MODELED.
pub struct Sn50;

impl Backend for Sn50 {
    fn name(&self) -> &'static str {
        "sn50"
    }

    fn bring_up(&self, spec: &ModelSpec, checkpoint: &Path, ctx: usize, out: &Path) -> Result<Brought, Refused> {
        use crate::compile::{compile_model_to_rdu_streamed, RduModel, StepShape};
        use crate::spatial::rdu_chip::RduChip;
        let mut source = crate::model::open_checkpoint(checkpoint).map_err(|e| Refused(format!("{}: {e}", checkpoint.display())))?;
        let model = RduModel { spec: spec.clone(), ..RduModel::from_checkpoint(&spec.arch, source.as_mut()).map_err(Refused)? };
        let mut files = Vec::new();
        let mut written_files: Vec<(PathBuf, String)> = Vec::new();
        let bundle = compile_model_to_rdu_streamed(&model, RduChip::Sn50, "weights", Some(StepShape { ctx, batch: 1, tp: 1, prefill: None }), |f| {
            let dir = out.join(&f.dir);
            written_files.extend(f.files().map(|(name, text)| (dir.join(name), text.to_string())));
            Ok(())
        })
        .map_err(Refused)?;
        for (path, text) in written_files {
            write(path, text, &mut files)?;
        }
        for (path, bytes) in &bundle.constants {
            write(out.join(path), bytes, &mut files)?;
        }
        write(out.join("manifest.json"), &bundle.manifest, &mut files)?;
        write(out.join("program.lg"), &bundle.program, &mut files)?;
        let whole = written(spec, Cut::Decode { ctx }).1;
        manifest_names(&bundle.manifest, &whole)?;
        Ok(Brought { programs: vec![whole], files })
    }

    fn run(&self, _spec: &ModelSpec, _checkpoint: &Path, _ctx: usize, _out: &Path, _sequences: &[Sequence]) -> Result<Generated, Refused> {
        Err(Refused("the SN50 runs its bundle under SambaNova's SDK, which is gated; no SN50 on this machine".to_string()))
    }
}

/// The spatial fabric the compiler generates for an FPGA: it does not lower a model the author writes yet, and says so.
pub struct Fabric;

impl Fabric {
    fn not_yet() -> Refused {
        Refused("the fabric does not lower a model the author writes yet".to_string())
    }
}

impl Backend for Fabric {
    fn name(&self) -> &'static str {
        "fabric"
    }

    fn bring_up(&self, _spec: &ModelSpec, _checkpoint: &Path, _ctx: usize, _out: &Path) -> Result<Brought, Refused> {
        Err(Fabric::not_yet())
    }

    fn run(&self, _spec: &ModelSpec, _checkpoint: &Path, _ctx: usize, _out: &Path, _sequences: &[Sequence]) -> Result<Generated, Refused> {
        Err(Fabric::not_yet())
    }
}

/// Whether a bundle's manifest names `whole`, the decode step it was lowered from, as its `program`: the identity a
/// backend reports is the one its artifact carries.
fn manifest_names(manifest: &str, whole: &Identity) -> Result<(), Refused> {
    let m: serde_json::Value = serde_json::from_str(manifest).map_err(|e| Refused(format!("the manifest: {e}")))?;
    let Some(line) = m["program"].as_str() else {
        return Err(Refused("the manifest names no program".to_string()));
    };
    if line != whole.line() {
        return Err(Refused(format!("the bundle names `{line}`, not the decode step `{}`", whole.line())));
    }
    Ok(())
}

/// Every chip a model is brought up on, in the order the command line lists them: the CPU tiers, then the accelerators.
pub fn backends() -> Vec<Box<dyn Backend>> {
    vec![
        Box::new(Vm),
        Box::new(Jit),
        Box::new(C),
        Box::new(Rust),
        Box::new(RiscvQemu),
        Box::new(Wasm),
        Box::new(Cuda),
        Box::new(D5005),
        Box::new(Sn50),
        Box::new(Fabric),
    ]
}

/// A checkpoint brought up: its spec, the identity of its decode step (the program every backend's cuts are cuts
/// of), its text, and each backend asked for with what it made or why it refused.
pub struct Bringup {
    pub spec: ModelSpec,
    pub decode: Identity,
    pub program: String,
    pub results: Vec<(&'static str, Result<Brought, Refused>)>,
}

/// Bring the checkpoint at `checkpoint` — a GGUF file, or a Hugging Face directory (`config.json` beside its
/// safetensors) — up on the backends named in `targets` (every one when empty), over caches of `ctx` positions, each
/// backend's artifacts under `out/<name>/` and the model's program as `out/program.lg`: the spec read from the
/// checkpoint's own metadata ([`bring_up_spec`]).
pub fn bring_up(checkpoint: &Path, ctx: usize, out: &Path, targets: &[String]) -> Result<Bringup, Refused> {
    let source = crate::model::open_checkpoint(checkpoint).map_err(io(checkpoint))?;
    let spec = ModelSpec::from_source(source.as_ref()).map_err(|e| Refused(format!("{}: {e}", checkpoint.display())))?;
    bring_up_spec(spec, checkpoint, ctx, out, targets)
}

/// [`bring_up`] for `spec`, whose weights the checkpoint at `checkpoint` holds.
pub fn bring_up_spec(spec: ModelSpec, checkpoint: &Path, ctx: usize, out: &Path, targets: &[String]) -> Result<Bringup, Refused> {
    crate::model::author::check_at(&spec, ctx).map_err(|e| Refused(format!("the language cannot write this model yet: {e}")))?;
    let unknown: Vec<&String> = targets.iter().filter(|t| !backends().iter().any(|b| b.name() == t.as_str())).collect();
    if !unknown.is_empty() {
        let known: Vec<&str> = backends().iter().map(|b| b.name()).collect();
        return Err(Refused(format!("no backend named {unknown:?}; the backends are {known:?}")));
    }
    let ((program, _, _), decode) = written(&spec, Cut::Decode { ctx });
    std::fs::create_dir_all(out).map_err(io(out))?;
    std::fs::write(out.join("program.lg"), &program).map_err(io(out))?;
    let results = backends()
        .into_iter()
        .filter(|b| targets.is_empty() || targets.iter().any(|t| t == b.name()))
        .map(|b| (b.name(), b.bring_up(&spec, checkpoint, ctx, &out.join(b.name()))))
        .collect();
    Ok(Bringup { spec, decode, program, results })
}

/// The runs of a model brought up: the prompt, what they are judged against, and each backend that brought the model
/// up with what it generated or why it did not run here.
pub struct Runs {
    pub prompt: Vec<u32>,
    pub reference: Reference,
    pub results: Vec<(&'static str, Result<Ran, Refused>)>,
}

/// Run the model `b` brought up on every backend that brought it up, `prompt` first, for as many tokens as
/// `reference` holds, in the order the backends were asked for: [`run_sequences`] of the one sequence.
pub fn run(b: &Bringup, checkpoint: &Path, ctx: usize, out: &Path, prompt: &[u32], reference: Reference) -> Runs {
    let one = [Sequence { prompt: prompt.to_vec(), tokens: reference.tokens.clone() }];
    let results = run_sequences(b, checkpoint, ctx, out, &one)
        .results
        .into_iter()
        .map(|(chip, r)| (chip, r.map(|g| Ran { tokens: g.tokens.into_iter().next().unwrap_or_default(), seconds: g.seconds, on: g.on })))
        .collect();
    Runs { prompt: prompt.to_vec(), reference, results }
}

/// The runs of several sequences: the sequences, and each backend that brought the model up with what it generated for
/// them or why it did not run here.
pub struct SequenceRuns {
    pub sequences: Vec<Sequence>,
    pub results: Vec<(&'static str, Result<Generated, Refused>)>,
}

/// Run the model `b` brought up on every backend that brought it up, every one of `sequences` from an empty state, in
/// the order the backends were asked for.
pub fn run_sequences(b: &Bringup, checkpoint: &Path, ctx: usize, out: &Path, sequences: &[Sequence]) -> SequenceRuns {
    let all = backends();
    let results = b
        .results
        .iter()
        .filter(|(_, brought)| brought.is_ok())
        .filter_map(|(name, _)| all.iter().find(|x| x.name() == *name))
        .map(|backend| (backend.name(), backend.run(&b.spec, checkpoint, ctx, &out.join(backend.name()), sequences)))
        .collect();
    SequenceRuns { sequences: sequences.to_vec(), results }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/bringup/tests.rs"]
mod tests;
