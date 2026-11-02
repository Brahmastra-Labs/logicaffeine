#!/usr/bin/env python3
"""Assemble the AOT-native run-only timing harness.

Each `largo emit rust` output is a whole program: some helper `fn`s, then `fn main()` (spawns
`_logos_main` on a big stack), then `fn _logos_main()` which does the compute and ends in `show(...)`.
To time the COMPILED tier run-only (fair vs the interp/VM in-process min-of-N and vs the on-chip run-only
cycle counter), we put EACH benchmark in its own module `mod bench_<name>` — its helper fns verbatim, and
`_logos_main`'s body as `pub fn run()` with every `show(` replaced by `core::hint::black_box(` (keeps the
result live so rustc -O3 can't dead-code-eliminate the computation, without printing during timing). One
crate, per-benchmark modules (no helper-name collisions), a min-of-N harness on a 256 MB stack (recursion).

  scripts/bench/build-aot-timing-harness.py <aot_dir_with_emitted_rs> <names...>  > harness.rs
"""
import re, sys, os

def fn_block_end(src, brace_open):
    """Return the index one past the closing brace of a body whose `{` is at `brace_open`."""
    depth, j = 1, brace_open + 1
    while depth:
        c = src[j]
        depth += (c == "{") - (c == "}")
        j += 1
    return j

def module_for(path, name):
    """The WHOLE emitted file becomes `mod bench_<name>` (all helpers verbatim, incl. unsafe/target_feature
    SIMD), with `fn main` removed and `fn _logos_main` renamed to `pub fn run` (its `show(` → black_box)."""
    src = open(path).read()
    # remove `fn main() { ... }` (the thread-spawn wrapper — the harness supplies its own big-stack thread)
    m = re.search(r"(?m)^fn\s+main\s*\(\s*\)\s*\{", src)
    if m:
        end = fn_block_end(src, src.index("{", m.end() - 1))
        src = src[:m.start()] + src[end:]
    # `_logos_main` -> the module's `pub fn run`
    src = src.replace("fn _logos_main()", "pub fn run()")
    # keep the compute live without printing during the timing loop
    src = src.replace("show(", "core::hint::black_box(")
    return (f"#[allow(unused_imports, unused_variables, unused_mut, non_snake_case, unused_parens, dead_code)]\n"
            f"mod bench_{name} {{\n{src}\n}}")

def main():
    aot_dir = sys.argv[1]
    names = sys.argv[2:]
    out = [module_for(os.path.join(aot_dir, f"{n}.rs"), n) for n in names]
    out.append("""
fn time_ns(f: fn(), iters: usize) -> u128 {
    f(); // warm
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        f();
        best = best.min(t.elapsed().as_nanos());
    }
    best
}
fn run() {
    let benches: &[(&str, fn())] = &[""")
    for n in names:
        out.append(f'        ("{n}", bench_{n}::run as fn()),')
    out.append("""    ];
    for (name, f) in benches {
        println!("{}|{}", name, time_ns(*f, 2000));
    }
}
fn main() {
    std::thread::Builder::new().stack_size(256 * 1024 * 1024).spawn(run).unwrap().join().unwrap();
}
""")
    print("\n".join(out))

if __name__ == "__main__":
    main()
