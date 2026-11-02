//! LOGOS source → what the whole-program fabric engine loads: the program lowered to an engine config, proven to
//! answer what the VM answers on the exact arguments about to be streamed before a byte is emitted, checked against
//! the deployed engine's profile and the flashed image's caps. One path for every caller — the `emit_wpe_soc`
//! example the board hosts compile through, and the Fabric Terminal's library — so a program is lowered the same
//! way wherever it runs.

use std::panic::{catch_unwind, AssertUnwindSafe};

use super::fabric_rtl::program_cfg_fabric;
use std::collections::{HashMap, HashSet};

use super::chip_protocol::{TAG_FLOAT, TAG_TEXT};
use super::wprog_engine_rtl::{
    eval_wprog_config, eval_wprog_config_run, func_code_needs_fpu, program_calls_fabric, program_main_fabric,
    program_output, serialize_program, wprog_config_fits_engine, wprog_config_fits_hw_engine,
    wprog_config_fits_stream_engine, wprog_config_image_unchecked, wprog_nregs, WBlock, WCell, WProgConfig, WSrc,
    WTerm,
};
use crate::compile::vm_outcome;

/// A way to lift a program to an engine config. A lifter panics on a program it cannot take; [`lower`] tries the
/// function lifters in order, so a caller that runs one lifter at a time (a process or an instance per attempt)
/// takes the same path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifter {
    /// The single-function CFG lifter.
    Cfg,
    /// The multi-function lifter: `Call` terminators and the engine's frame stack, for recursion.
    Calls,
    /// The whole program from its `## Main`, each `Show` a record the engine streams.
    Main,
}

impl Lifter {
    /// The lifters a function is tried with, in order.
    pub const FUNCTION: [Lifter; 2] = [Lifter::Cfg, Lifter::Calls];

    /// The name a host reads (`lifter=` in `emit_wpe_soc`'s facts).
    pub fn name(self) -> &'static str {
        match self {
            Lifter::Cfg => "cfg",
            Lifter::Calls => "calls",
            Lifter::Main => "main",
        }
    }

    /// Lift `func` of `src` — or, for [`Lifter::Main`], its `## Main`.
    pub fn lift(self, src: &str, func: &str) -> WProgConfig {
        match self {
            Lifter::Cfg => serialize_program(&program_cfg_fabric(src, func)),
            Lifter::Calls => program_calls_fabric(src, func),
            Lifter::Main => program_main_fabric(src),
        }
    }
}

/// The deployed engine a program must fit. A `.wire` carries no record of the profile it was built for, and an
/// over-cap image loads fine and returns a plausible wrong number, so this check is the guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// The XC7A100T engine on Board B.
    Hw,
    /// The XC7A200T block-RAM engine on Board A.
    Stream,
    /// The generic engine's caps.
    Generic,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::Hw, Profile::Stream, Profile::Generic];

    pub fn name(self) -> &'static str {
        match self {
            Profile::Hw => "hw",
            Profile::Stream => "stream",
            Profile::Generic => "generic",
        }
    }

    pub fn from_name(name: &str) -> Option<Profile> {
        Profile::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Whether `cfg`'s static dimensions fit this engine; the refusal names the cap it exceeds.
    pub fn fits(self, cfg: &WProgConfig) -> Result<(), String> {
        match self {
            Profile::Hw => wprog_config_fits_hw_engine(cfg),
            Profile::Stream => wprog_config_fits_stream_engine(cfg),
            Profile::Generic => wprog_config_fits_engine(cfg),
        }
    }
}

/// Run `f` under a lowering attempt with the panic hook silenced: a lifter that cannot handle a program panics,
/// and that is an ordinary "try the next lifter" outcome here, not a crash worth a backtrace on stderr.
fn attempt<T>(f: impl FnOnce() -> T) -> Option<T> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = catch_unwind(AssertUnwindSafe(f)).ok();
    std::panic::set_hook(hook);
    r
}

/// The VM's answer for `func(params)` — a synthesized `## Main` showing the call, run on the reference VM (the
/// same `vm_outcome` every execution tier is locked against). This is the golden the chip must reproduce.
pub fn vm_golden(src: &str, func: &str, params: &[i64]) -> Result<i64, String> {
    let al = params
        .iter()
        .map(|a| if *a < 0 { format!("(0 - {})", -a) } else { a.to_string() })
        .collect::<Vec<_>>()
        .join(", ");
    let prog = format!("{src}## Main\n    Show {func}({al}).\n");
    let out = vm_outcome(&prog);
    if let Some(e) = out.error {
        return Err(format!("the VM cannot run {func}({al}): {e}"));
    }
    out.output
        .split_whitespace()
        .next()
        .and_then(|w| w.parse::<i64>().ok())
        .ok_or_else(|| format!("{func}({al}) did not show an Int on the VM: {:?}", out.output))
}

/// The VM's OUTPUT for a whole program — one line per `Show` — the golden a Main program's streamed records are
/// checked against.
pub fn vm_output_lines(src: &str) -> Result<Vec<String>, String> {
    let out = vm_outcome(src);
    if let Some(e) = out.error {
        return Err(format!("the VM cannot run the program: {e}"));
    }
    Ok(out.output.trim_end().lines().map(str::to_string).collect())
}

/// Lower a whole `## Main` program and PROVE the engine model prints exactly what the VM prints before a byte
/// is emitted: the output tape rendered line by line == `vm_outcome(src).output`.
pub fn lower_main(src: &str) -> Result<(WProgConfig, Vec<String>), String> {
    let vm = vm_output_lines(src)?;
    let cfg = attempt(|| Lifter::Main.lift(src, "")).ok_or_else(|| "the Main lifter refused the program".to_string())?;
    let model = attempt(|| program_output(&cfg, &[])).ok_or_else(|| "the engine model faulted".to_string())?;
    if model != vm {
        return Err(format!("the engine model's output {model:?} is not the VM's {vm:?}"));
    }
    Ok((cfg, vm))
}

/// Lower `func` to an engine config the same way the benchmark lane does: the single-function CFG lifter when
/// it agrees with the VM on these params, else the multi-function (call-stack) lifter. Either way the config is
/// PROVEN ≡ VM on the exact params about to be streamed before a byte is emitted.
pub fn lower(src: &str, func: &str, params: &[i64]) -> Result<(WProgConfig, Lifter, i64), String> {
    let vm = vm_golden(src, func, params)?;
    let mut tried = Vec::new();
    for lifter in Lifter::FUNCTION {
        let name = lifter.name();
        match attempt(|| lifter.lift(src, func)) {
            Some(cfg) => match attempt(|| eval_wprog_config(&cfg, params)) {
                Some(got) if got == vm => return Ok((cfg, lifter, vm)),
                Some(got) => tried.push(format!("{name} lifter gives {got}")),
                None => tried.push(format!("{name} lifter's model faulted")),
            },
            None => tried.push(format!("{name} lifter refused the program")),
        }
    }
    Err(format!("no lowering runs {func}({params:?}) ≡ VM (VM says {vm}; {})", tried.join("; ")))
}

/// Check the program against the caps of the image that is actually on the board (`NAME=N,…`, the recipe the
/// build records beside every bitstream). The static dimensions are read off the config; DEPTH and HEAP are
/// data-dependent, so they are MEASURED by running the program on the model with the exact params about to be
/// streamed. An over-cap program loads fine and returns a plausible wrong number — this is the guard.
pub fn fits_caps(cfg: &WProgConfig, params: &[i64], caps: &str) -> Result<(), String> {
    let run = eval_wprog_config_run(cfg, params, &[]);
    let im = wprog_config_image_unchecked(cfg);
    let measured: Vec<(&str, usize)> = vec![
        ("NREG", wprog_nregs(cfg)),
        ("MAXBLK", cfg.blocks.len()),
        ("MAXCELL", cfg.blocks.iter().map(|b| b.cells.len()).sum()),
        ("MAXV", cfg.blocks.iter().map(|b| b.cells.len()).max().unwrap_or(0)),
        ("MAXWR", cfg.blocks.iter().map(|b| b.writes.len()).sum()),
        ("MAXLIV", cfg.blocks.iter().map(|b| b.livein.len()).sum()),
        ("MAXARG", im.arg_op.len()),
        ("MAXK", im.konst.len()),
        ("HEAP", run.heap.len()),
        ("DEPTH", run.peak_depth),
    ];
    for entry in caps.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (name, cap) = entry.split_once('=').ok_or_else(|| format!("bad --caps entry {entry:?} (want NAME=N)"))?;
        let cap: usize = cap.trim().parse().map_err(|_| format!("bad --caps value in {entry:?}"))?;
        // `FPU=0`: the image was built without the float lane — a program with any float functional unit has no
        // datapath to run on and must be refused, not streamed to return a zero.
        if name.trim() == "FPU" {
            if cap == 0 {
                if let Some(c) =
                    cfg.blocks.iter().flat_map(|b| b.cells.iter()).find(|c| c.kind == 2 && func_code_needs_fpu(c.code))
                {
                    return Err(format!(
                        "the program uses a float unit (func code {}) but the flashed image was built with FPU=0",
                        c.code
                    ));
                }
            }
            continue;
        }
        let need = measured.iter().find(|(n, _)| *n == name.trim()).map(|(_, v)| *v).ok_or_else(|| {
            format!("unknown cap {name:?} (known: NREG MAXBLK MAXCELL MAXV MAXWR MAXLIV MAXARG MAXK HEAP DEPTH)")
        })?;
        if need > cap {
            return Err(format!("{name} needs {need} but the flashed image has {name}={cap}"));
        }
    }
    Ok(())
}

// ── the fabric map ─────────────────────────────────────────────────────────────────────────────────────────────

/// What each part of the fabric does for one compiled program, read off the config the engine runs: the labels the
/// Fabric Terminal draws beside its activity panel, whose BLOCKS tiles are the config's blocks and whose CELLS strip
/// is its cells in the engine's flat order.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct FabricMap {
    pub blocks: Vec<MapBlock>,
}

/// One block of the config: a BLOCKS tile.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct MapBlock {
    /// Its part in the control flow: `init` (where the program begins), `fn` (a function's entry, entered by a call),
    /// `test` (it branches), `loop` (it jumps back to an earlier block), `step` (it jumps forward), `call` (it calls a
    /// function), `end` (it returns), `idle` (no path reaches it, so it never runs).
    pub role: &'static str,
    /// Its first cell's place in the CELLS strip: the cells of block `b` follow those of blocks `0..b`.
    pub first_cell: usize,
    /// Each of its cells' operation, in order.
    pub cells: Vec<String>,
    /// What it does before control moves on, a statement each, over the program's variable names: what it shows
    /// and stores, the lists and texts it builds, and the registers it writes that some block reads.
    pub says: Vec<String>,
    /// Where control goes next.
    pub next: MapNext,
}

/// A block's terminator, as the map gives it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MapNext {
    Jump { to: usize },
    Branch { cond: String, then: usize, otherwise: usize },
    Return { value: String },
    Call { callee: usize, args: Vec<String>, result: String, back: usize },
}

/// The Main frame's variable names by register — the registers a [`Lifter::Main`] config's own blocks read and
/// write. A function's blocks have registers of their own frame, which these names do not describe.
pub fn main_register_names(src: &str) -> HashMap<u16, String> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let Ok((stmts, _types, _policies)) = parsed else {
            return HashMap::new();
        };
        crate::vm::Compiler::compile_for_debug(stmts, interner, None)
            .map(|p| p.reg_names.into_iter().collect())
            .unwrap_or_default()
    })
}

const ARITH: [&str; 9] = ["+", "-", "*", "&", "|", "^", "<<", ">>", ">>>"];
const COMPARE: [&str; 6] = ["<", ">", "<=", ">=", "==", "!="];
const UNIT: [&str; 21] = [
    "/", "%", "//", "**", "*hi", "f+", "f-", "f*", "f/", "sqrt", "floor", "~=", "*", "float", "f<", "f>", "f<=", "f>=",
    "f==", "f!=", "popcount",
];

/// Whether functional unit `code` works on floats (its operands are f64 bit patterns).
fn float_unit(code: u8) -> bool {
    matches!(code, 5..=11 | 14..=19)
}

/// A cell's operation, as the CELLS strip's label for it.
fn cell_op(c: &WCell) -> String {
    let pick = |table: &[&str]| table.get(c.code as usize).copied().unwrap_or("?").to_string();
    match c.kind {
        0 => pick(&ARITH),
        1 => pick(&COMPARE),
        2 => pick(&UNIT),
        3 => "ld".into(),
        4 => "st".into(),
        5 => "new".into(),
        6 => "copy".into(),
        7 => "++".into(),
        8 => "at".into(),
        9 => (if c.code == 0 { "==" } else { "!=" }).into(),
        10 => "show".into(),
        _ => "?".into(),
    }
}

/// The map of `cfg`, with `names` for the registers of the blocks [`Lifter::Main`]'s `## Main` region runs — the
/// blocks reached from block 0 without entering a call.
pub fn fabric_map(cfg: &WProgConfig, names: &HashMap<u16, String>) -> FabricMap {
    let reach = |through_calls: bool| {
        let mut seen = HashSet::new();
        let mut work = vec![0usize];
        while let Some(i) = work.pop() {
            if i >= cfg.blocks.len() || !seen.insert(i) {
                continue;
            }
            match &cfg.blocks[i].term {
                WTerm::Jump(t) => work.push(*t),
                WTerm::Branch { then_blk, else_blk, .. } => work.extend([*then_blk, *else_blk]),
                WTerm::Call { callee_block, ret_block, .. } => {
                    work.push(*ret_block);
                    if through_calls {
                        work.push(*callee_block);
                    }
                }
                WTerm::Return(_) => {}
            }
        }
        seen
    };
    let (main, runs) = (reach(false), reach(true));
    let callees: HashSet<usize> = cfg
        .blocks
        .iter()
        .filter_map(|b| match &b.term {
            WTerm::Call { callee_block, .. } => Some(*callee_block),
            _ => None,
        })
        .collect();
    // A register some block reads; a write to any other is the VM's scratch, with no effect on the program.
    let read: HashSet<u16> = cfg.blocks.iter().flat_map(|b| b.livein.iter().copied()).collect();
    // A register whose value a float unit or a float Show reads: a constant written to it is an f64's bits.
    let float_regs: HashSet<u16> = cfg
        .blocks
        .iter()
        .flat_map(|b| {
            b.cells.iter().flat_map(move |c| {
                let float = (c.kind == 2 && float_unit(c.code)) || (c.kind == 10 && c.code == TAG_FLOAT);
                c.ops.iter().filter(move |s| float && s.mode == 0).filter_map(move |s| b.livein.get(s.reference as usize).copied())
            })
        })
        .collect();
    let none = HashMap::new();
    let mut first = 0;
    let blocks = cfg
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let role = if !runs.contains(&i) {
                "idle"
            } else if i == 0 {
                "init"
            } else if callees.contains(&i) {
                "fn"
            } else {
                match &b.term {
                    WTerm::Return(_) => "end",
                    WTerm::Call { .. } => "call",
                    WTerm::Branch { .. } => "test",
                    WTerm::Jump(t) if *t <= i => "loop",
                    WTerm::Jump(_) => "step",
                }
            };
            let text = BlockText::new(b, if main.contains(&i) { names } else { &none }, &float_regs);
            let block = MapBlock {
                role,
                first_cell: first,
                cells: b.cells.iter().map(cell_op).collect(),
                says: text.says(&read),
                next: text.next(),
            };
            first += b.cells.len();
            block
        })
        .collect();
    FabricMap { blocks }
}

/// A list or text the block builds cell by cell: its elements, and the register it is kept in.
struct Built {
    items: Vec<WSrc>,
    text: Option<String>,
    kept_in: Option<u16>,
}

/// One block written out: its cells as expressions over its live-in registers, then its statements and terminator.
struct BlockText<'a> {
    b: &'a WBlock,
    names: &'a HashMap<u16, String>,
    float_regs: &'a HashSet<u16>,
    built: HashMap<usize, Built>,
    consumed: HashSet<usize>,
}

impl<'a> BlockText<'a> {
    fn new(b: &'a WBlock, names: &'a HashMap<u16, String>, float_regs: &'a HashSet<u16>) -> Self {
        let mut t = BlockText { b, names, float_regs, built: HashMap::new(), consumed: HashSet::new() };
        let is_val = |s: &WSrc, i: usize| s.mode == 2 && s.reference as usize == i;
        let konst = |s: &WSrc| (s.mode == 1).then_some(s.konst);
        for (a, c) in b.cells.iter().enumerate() {
            let Some(size) = (c.kind == 5).then(|| konst(&c.ops[0])).flatten() else { continue };
            let n = size - 1;
            let Some(len_write) =
                b.cells.iter().position(|w| w.kind == 4 && is_val(&w.ops[0], a) && konst(&w.ops[1]) == Some(n))
            else {
                continue;
            };
            let mut used = vec![len_write];
            let mut items = Vec::new();
            for j in 1..=n {
                let add = b.cells.iter().position(|x| {
                    x.kind == 0 && x.code == 0 && is_val(&x.ops[0], a) && konst(&x.ops[1]) == Some(j)
                });
                let write = add.and_then(|k| b.cells.iter().position(|w| w.kind == 4 && is_val(&w.ops[0], k)));
                match (add, write) {
                    (Some(k), Some(w)) => {
                        used.extend([k, w]);
                        items.push(b.cells[w].ops[1]);
                    }
                    _ => break,
                }
            }
            if items.len() as i64 != n {
                continue;
            }
            let read_as_text = b.cells.iter().any(|u| {
                let reads = |slot: usize| is_val(&u.ops[slot], a);
                (u.kind == 10 && u.code == TAG_TEXT && reads(0))
                    || (u.kind == 7 && ((u.code & 1 != 0 && reads(0)) || (u.code & 2 != 0 && reads(1))))
                    || (matches!(u.kind, 8 | 9) && (reads(0) || (u.kind == 9 && reads(1))))
            });
            let bytes: Option<Vec<i64>> = items.iter().map(konst).collect();
            let text = bytes.filter(|v| read_as_text && v.iter().all(|&x| (0x20..0x7F).contains(&x))).map(|v| {
                format!("\"{}\"", v.iter().map(|&x| x as u8 as char).collect::<String>())
            });
            let kept_in = b.writes.iter().find(|(_, s)| is_val(s, a)).map(|(r, _)| *r);
            t.built.insert(a, Built { items, text, kept_in });
            t.consumed.extend(used);
        }
        t
    }

    fn reg(&self, r: u16) -> String {
        self.names.get(&r).cloned().unwrap_or_else(|| format!("r{r}"))
    }

    fn konst(&self, k: i64, float: bool) -> String {
        if float {
            let f = f64::from_bits(k as u64);
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{f:.1}")
            } else {
                format!("{f}")
            }
        } else {
            k.to_string()
        }
    }

    fn src_as(&self, s: &WSrc, float: bool) -> String {
        match s.mode {
            0 => self.b.livein.get(s.reference as usize).map_or_else(|| "?".into(), |&r| self.reg(r)),
            1 => self.konst(s.konst, float),
            _ => self.val(s.reference as usize),
        }
    }

    fn src(&self, s: &WSrc) -> String {
        self.src_as(s, false)
    }

    /// The literal a built list or text is written as.
    fn literal(&self, built: &Built) -> String {
        built.text.clone().unwrap_or_else(|| format!("[{}]", built.items.iter().map(|s| self.src(s)).collect::<Vec<_>>().join(", ")))
    }

    /// Word `j` of the list or text at `base`, or its length (word 0), as `base[j]` / `length(base)`.
    fn element(&self, at: &WSrc) -> (String, Option<String>) {
        if at.mode == 2 {
            if let Some(c) = self.b.cells.get(at.reference as usize) {
                if c.kind == 0 && c.code == 0 {
                    return (self.src(&c.ops[0]), Some(self.src(&c.ops[1])));
                }
            }
        }
        (self.src(at), None)
    }

    fn val(&self, i: usize) -> String {
        if let Some(built) = self.built.get(&i) {
            return built.kept_in.map(|r| self.reg(r)).unwrap_or_else(|| self.literal(built));
        }
        let Some(c) = self.b.cells.get(i) else { return format!("v{i}") };
        let float = c.kind == 2 && float_unit(c.code);
        let (a, b) = (self.src_as(&c.ops[0], float), self.src_as(&c.ops[1], float));
        let op = cell_op(c);
        let e = match c.kind {
            0 | 7 | 9 => bin(&a, &op, &b),
            1 => {
                let cond = bin(&a, &op, &b);
                if c.ops[2].mode == 1 && c.ops[2].konst == 1 && c.ops[3].mode == 1 && c.ops[3].konst == 0 {
                    cond
                } else {
                    format!("({cond} ? {} : {})", self.src(&c.ops[2]), self.src(&c.ops[3]))
                }
            }
            2 => match c.code {
                9 | 10 | 13 | 20 => format!("{op}({a})"),
                _ => bin(&a, &op, &b),
            },
            3 => match self.element(&c.ops[0]) {
                (base, Some(j)) => format!("{base}[{j}]"),
                (base, None) => format!("length({base})"),
            },
            4 => b,
            5 => format!("new[{a}]"),
            6 => format!("copy({a})"),
            8 => format!("{a}[{b}]"),
            10 => a,
            _ => format!("v{i}"),
        };
        if e.len() > 48 {
            format!("v{i}")
        } else {
            e
        }
    }

    /// The block's statements, in the order its cells and writes take effect; `read` holds the registers some block
    /// reads, the only writes with an effect.
    fn says(&self, read: &HashSet<u16>) -> Vec<String> {
        let mut out = Vec::new();
        let mut defined = HashSet::new();
        for (i, c) in self.b.cells.iter().enumerate() {
            if let Some(built) = self.built.get(&i) {
                if let Some(r) = built.kept_in {
                    out.push(format!("{} = {}", self.reg(r), self.literal(built)));
                    defined.insert(r);
                }
                continue;
            }
            if self.consumed.contains(&i) {
                continue;
            }
            match c.kind {
                10 => out.push(format!("show {}", self.src_as(&c.ops[0], c.code == TAG_FLOAT))),
                4 => out.push(match self.element(&c.ops[0]) {
                    (base, Some(j)) => format!("{base}[{j}] = {}", self.src(&c.ops[1])),
                    (base, None) => format!("length({base}) = {}", self.src(&c.ops[1])),
                }),
                _ => {}
            }
        }
        for (r, s) in &self.b.writes {
            if defined.contains(r) || !read.contains(r) {
                continue;
            }
            let (name, value) = (self.reg(*r), self.src_as(s, self.float_regs.contains(r)));
            if name != value {
                out.push(format!("{name} = {value}"));
            }
        }
        out
    }

    fn next(&self) -> MapNext {
        match &self.b.term {
            WTerm::Jump(t) => MapNext::Jump { to: *t },
            WTerm::Branch { cond, then_blk, else_blk } => {
                MapNext::Branch { cond: self.src(cond), then: *then_blk, otherwise: *else_blk }
            }
            WTerm::Return(s) => MapNext::Return { value: self.src(s) },
            WTerm::Call { callee_block, args, dst_reg, ret_block } => MapNext::Call {
                callee: *callee_block,
                args: args.iter().map(|a| self.src(a)).collect(),
                result: self.reg(*dst_reg),
                back: *ret_block,
            },
        }
    }
}

fn bin(a: &str, op: &str, b: &str) -> String {
    let wrap = |x: &str| {
        if x.contains(' ') && !x.starts_with('"') && !x.starts_with('(') && !x.starts_with('[') {
            format!("({x})")
        } else {
            x.to_string()
        }
    };
    format!("{} {op} {}", wrap(a), wrap(b))
}
