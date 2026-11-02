#!/usr/bin/env python3
"""Makes a sokonanoda-family rival report per-declaration instruction costs, like
`logos_kernel --decl-costs`, for a declaration-by-declaration comparison with ours.

    scripts/lean/arena-crown/rival-decl-costs.py <source tree copy>

Patches the copy in place (never the fetched tree): every checking thread counts its own user-space
instructions (`perf_event_open`, bound to the performance-core PMU) around each declaration it
checks and, when MATHGRAPH_DECL_COSTS=<prefix> is set, writes `<prefix>.<thread>` with the columns
`index instructions kind group name`. `group` is the first type of the declaration's inductive
block (for inductive types, constructors and recursors) or the name itself — our checker reports a
whole inductive record as one declaration named by its first type, and `diff-decls.py` sums the
rival's rows by group to match. Unset, the hook costs one branch per declaration.

Two source layouts are recognised:

  * `src/tc.rs` (sokonanoda to 10-01, mathgraph `flash`): one loop checks every declaration, an
    inductive block's types, constructors and recursors among them.
  * `src/checker/tc.rs` (sokonanoda from 10-05): the export is parsed whole, then one thread checks
    every inductive block and quotient in order (`check_inductives`, one row per block, kind
    `inductive` or `quot`), then the checking threads take the remaining declarations in chunks
    (the members of a block pass through them unchecked and get no row). Parsing is in no row: its
    cost is the rival's `--parse-only` total.
"""
import pathlib
import sys

COUNTER = r'''//! Per-declaration instruction costs (a measurement patch; see scripts/lean/arena-crown/rival-decl-costs.py).
use std::io::Write;

unsafe fn syscall5(n: usize, a: usize, b: usize, c: usize, d: usize, e: usize) -> isize {
    let r: isize;
    std::arch::asm!("syscall", inlateout("rax") n as isize => r, in("rdi") a, in("rsi") b, in("rdx") c,
        in("r10") d, in("r8") e, lateout("rcx") _, lateout("r11") _, options(nostack));
    r
}

pub(crate) struct CostLog {
    fd: i32,
    out: std::io::BufWriter<std::fs::File>,
    start: u64,
}

static THREAD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl CostLog {
    pub(crate) fn open() -> Option<CostLog> {
        let prefix = std::env::var("MATHGRAPH_DECL_COSTS").ok()?;
        let pmu = std::fs::read_to_string("/sys/bus/event_source/devices/cpu_core/type")
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok());
        let mut attr = [0u64; 8];
        attr[0] = 64u64 << 32;
        attr[1] = 1 | pmu.map_or(0, |t| t << 32);
        attr[5] = (1 << 5) | (1 << 6);
        let fd = unsafe { syscall5(298, attr.as_ptr() as usize, 0, usize::MAX, usize::MAX, 8) };
        assert!(fd >= 0, "perf_event_open failed (errno {})", -fd);
        let t = THREAD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let file = std::fs::File::create(format!("{prefix}.{t}")).expect("create the cost log");
        let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
        writeln!(out, "index\tinstructions\tkind\tgroup\tname").unwrap();
        Some(CostLog { fd: fd as i32, out, start: 0 })
    }

    fn read(&self) -> u64 {
        let mut v = 0u64;
        let n = unsafe { syscall5(0, self.fd as usize, &mut v as *mut u64 as usize, 8, 0, 0) };
        assert_eq!(n, 8);
        v
    }

    pub(crate) fn begin(&mut self) {
        self.start = self.read();
    }

    pub(crate) fn end(&mut self, index: usize, kind: &str, group: &str, name: &str) {
        let spent = self.read() - self.start;
        writeln!(self.out, "{index}\t{spent}\t{kind}\t{group}\t{name}").unwrap();
    }
}
'''

FLASH_DOTTED = r'''
pub(crate) fn dotted(n: crate::util::NamePtr<'_>) -> String {
    match n.as_ref().kind {
        crate::name::Name::Anon => String::new(),
        crate::name::Name::Str(p, s, _) => {
            let pre = dotted(p);
            let s: &str = s.as_ref();
            if pre.is_empty() { s.to_string() } else { format!("{pre}.{s}") }
        }
        crate::name::Name::Num(p, k, _) => {
            let pre = dotted(p);
            if pre.is_empty() { k.to_string() } else { format!("{pre}.{k}") }
        }
    }
}
'''

FLASH_LOOP_OLD = '''                while i < end {
                    let (_, d) = self.declars.get_index(i).expect("declaration index out of range");
                    i += 1;
                    self.check_declar_with(tctx, cache, sbump.get(), d);'''

FLASH_LOOP_NEW = '''                while i < end {
                    let (_, d) = self.declars.get_index(i).expect("declaration index out of range");
                    i += 1;
                    if let Some(log) = costs.as_mut() {
                        log.begin();
                        self.check_declar_with(tctx, cache, sbump.get(), d);
                        let (kind, group) = self.cost_group(d);
                        log.end(i - 1, kind, &crate::decl_costs::dotted(group), &crate::decl_costs::dotted(d.info().name));
                    } else {
                        self.check_declar_with(tctx, cache, sbump.get(), d);
                    }'''

FLASH_SESSION_OLD = '''        let mut pending = Some(first);
        loop {'''

FLASH_SESSION_NEW = '''        let mut pending = Some(first);
        let mut costs = crate::decl_costs::CostLog::open();
        loop {'''

FLASH_GROUP_FN = '''
    /// The declaration's kind and the first type of its inductive block (itself otherwise).
    fn cost_group(&self, d: &Declar<'p>) -> (&'static str, crate::util::NamePtr<'p>) {
        let block_first = |n: crate::util::NamePtr<'p>| match self.mutual_block_sizes.get(&n) {
            Some(&(start, _)) => self.declars.get_index(start).map_or(n, |(k, _)| *k),
            None => n,
        };
        match d {
            Declar::Inductive(ind) => ("inductive", block_first(ind.info.name)),
            Declar::Constructor(c) => ("ctor", block_first(c.inductive_name)),
            Declar::Recursor(r) => ("rec", r.all_inductives.first().map_or(r.info.name, |n| block_first(*n))),
            Declar::Definition { info, .. } => ("def", info.name),
            Declar::Theorem { info, .. } => ("thm", info.name),
            Declar::Opaque { info, .. } => ("opaque", info.name),
            Declar::Axiom { info } => ("axiom", info.name),
            Declar::Quot { info } => ("quot", info.name),
        }
    }

    fn run_session<F>(&self, first: (usize, usize), mut next_chunk: F)'''

CHECKER_SESSION_OLD = '''        let mut cache = TcCache::new();
        let mut pending = Some(first);
        loop {'''

CHECKER_SESSION_NEW = '''        let mut cache = TcCache::new();
        let mut pending = Some(first);
        let mut costs = crate::decl_costs::CostLog::open();
        loop {'''

CHECKER_LOOP_OLD = '''                    report.check(d, || self.check_declar_with(tctx, &mut cache, d));'''

CHECKER_LOOP_NEW = '''                    let block_member = matches!(
                        d,
                        crate::checker::env::Declar::Inductive(..)
                            | crate::checker::env::Declar::Constructor(..)
                            | crate::checker::env::Declar::Recursor(..)
                            | crate::checker::env::Declar::Quot { .. }
                    );
                    if let Some(log) = costs.as_mut().filter(|_| !block_member) {
                        log.begin();
                        report.check(d, || self.check_declar_with(tctx, &mut cache, d));
                        let name = self.display(d.info().name).to_string();
                        log.end(i - 1, d.kind(), &name, &name);
                    } else {
                        report.check(d, || self.check_declar_with(tctx, &mut cache, d));
                    }'''

INDUCTIVES_SETUP_OLD = '''                        let mut blocks = Vec::new();
                        let mut next = 0;'''

INDUCTIVES_SETUP_NEW = '''                        let mut blocks = Vec::new();
                        let mut next = 0;
                        let mut costs = crate::decl_costs::CostLog::open();'''

QUOT_OLD = '''                                crate::checker::env::Declar::Quot { .. } => {
                                    let mut cache = TcCache::new_small();
                                    let derived = crate::checker::quot::derive_quot(
                                        &mut ctx, &mut cache, declar,
                                    );
                                    (next, 1, vec![derived])
                                }'''

QUOT_NEW = '''                                crate::checker::env::Declar::Quot { .. } => {
                                    if let Some(log) = costs.as_mut() {
                                        log.begin();
                                    }
                                    let mut cache = TcCache::new_small();
                                    let derived = crate::checker::quot::derive_quot(
                                        &mut ctx, &mut cache, declar,
                                    );
                                    if let Some(log) = costs.as_mut() {
                                        let name = self.display(declar.info().name).to_string();
                                        log.end(next, "quot", &name, &name);
                                    }
                                    (next, 1, vec![derived])
                                }'''

INDUCTIVE_OLD = '''                                    let mut cache = TcCache::new_small();
                                    let derived =
                                        self.check_inductive_declar(&mut ctx, &mut cache, ind);
                                    (start, size, derived)'''

INDUCTIVE_NEW = '''                                    if let Some(log) = costs.as_mut() {
                                        log.begin();
                                    }
                                    let mut cache = TcCache::new_small();
                                    let derived =
                                        self.check_inductive_declar(&mut ctx, &mut cache, ind);
                                    if let Some(log) = costs.as_mut() {
                                        let first = self.declars.get_index(start).map_or(ind.info.name, |(k, _)| *k);
                                        let group = self.display(first).to_string();
                                        let name = self.display(ind.info.name).to_string();
                                        log.end(start, "inductive", &group, &name);
                                    }
                                    (start, size, derived)'''


def replace_once(text, old, new, what):
    if text.count(old) != 1:
        sys.exit(f"anchor for {what} matched {text.count(old)} times; the rival source moved")
    return text.replace(old, new)


def register(src, path):
    lib = (src / path).read_text()
    if "mod decl_costs;" not in lib:
        lib += "pub(crate) mod decl_costs;\n"
    (src / path).write_text(lib)


def patch_flash(src):
    (src / "decl_costs.rs").write_text(COUNTER + FLASH_DOTTED)
    register(src, "lib.rs")
    tc = (src / "tc.rs").read_text()
    tc = replace_once(tc, FLASH_LOOP_OLD, FLASH_LOOP_NEW, "the declaration loop")
    tc = replace_once(tc, FLASH_SESSION_OLD, FLASH_SESSION_NEW, "the session start")
    tc = replace_once(tc, "\n    fn run_session<F>(&self, first: (usize, usize), mut next_chunk: F)", FLASH_GROUP_FN, "run_session")
    (src / "tc.rs").write_text(tc)


def patch_checker(src):
    (src / "decl_costs.rs").write_text(COUNTER)
    register(src, "lib.rs")
    tc = (src / "checker" / "tc.rs").read_text()
    tc = replace_once(tc, CHECKER_SESSION_OLD, CHECKER_SESSION_NEW, "the session start")
    tc = replace_once(tc, CHECKER_LOOP_OLD, CHECKER_LOOP_NEW, "the declaration loop")
    (src / "checker" / "tc.rs").write_text(tc)
    ctx = (src / "checker" / "context.rs").read_text()
    ctx = replace_once(ctx, INDUCTIVES_SETUP_OLD, INDUCTIVES_SETUP_NEW, "the inductive pass")
    ctx = replace_once(ctx, QUOT_OLD, QUOT_NEW, "the quotient check")
    ctx = replace_once(ctx, INDUCTIVE_OLD, INDUCTIVE_NEW, "the inductive block check")
    (src / "checker" / "context.rs").write_text(ctx)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    root = pathlib.Path(sys.argv[1])
    src = root / "src"
    if (src / "checker" / "tc.rs").exists():
        patch_checker(src)
    elif (src / "tc.rs").exists():
        patch_flash(src)
    else:
        sys.exit(f"{root} is neither source layout this script knows")
    print(f"patched {root}")


if __name__ == "__main__":
    main()
