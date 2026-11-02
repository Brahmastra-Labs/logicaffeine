#!/usr/bin/env python3
"""Generate a stdlib-parity bench package from a compact per-capability spec.

Each spec supplies the amortizing-loop STEP in each language (the body that runs
once per iteration and folds into an accumulator `s`/`sum`), plus any helper
functions / imports. The generator wraps those in the standard main() skeleton for
each language, computes `expected` by running the Python reference, cross-checks
that C/Rust/Go/Python all agree, and writes the package + meta.json. LOGOS is
verified separately (largo build) by the caller.

Spec schema (one JSON object, or a list of them):
{
  "dir": "bits_leading_zeros",          # package dir under benchmarks/parity/bench/
  "capability": "bits.leading_zeros",   # catalog id (goes in meta.json + result name)
  "arg": "20000000",                    # N, passed as argv[1] to every program
  "beat": ["go","rust"],                # which langs LOGOS must beat (from catalog)
  "workload": "one-line human description",
  "loop": "linear",                     # linear: i in 1..=n ; triangular: i<=j<=n
  "logos_step": "Set sum to sum + leadingZeros(i, 64).",
  "c_step":     "s += __builtin_clzll((unsigned long long)i);",
  "rust_step":  "s += (i as u64).leading_zeros() as i64;",
  "go_step":    "s += int64(bits.LeadingZeros64(uint64(i)))",
  "py_step":    "s += 64 - i.bit_length()",
  # optional helpers (declarations placed above main / functions):
  "logos_helpers": "", "c_helpers": "", "rust_helpers": "", "go_imports": ["math/bits"],
  "py_helpers": "",
  # optional: skip a language entirely (not in `beat` and awkward to express)
  "skip": []                            # e.g. ["go"]
}
"""
import json, os, subprocess, sys, tempfile, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BENCH = os.path.join(ROOT, "benchmarks/parity/bench")


def logos_src(spec):
    inner = {
        "linear": "Let mutable i be 1.\nWhile i is at most n:\n    {step}\n    Set i to i + 1.",
        "triangular": "Let mutable i be 1.\nWhile i is at most n:\n    Let mutable j be i.\n    While j is at most n:\n        {step}\n        Set j to j + 1.\n    Set i to i + 1.",
    }[spec.get("loop", "linear")].format(step=spec["logos_step"])
    helpers = spec.get("logos_helpers", "")
    return (
        "## To native args () -> Seq of Text\n"
        "## To native parseInt (s: Text) -> Int\n"
        + (helpers + "\n" if helpers else "")
        + "\n## Main\n"
        "Let arguments be args().\n"
        "Let n be parseInt(item 2 of arguments).\n"
        "Let mutable sum be 0.\n"
        + inner + "\n"
        "Show sum.\n"
    )


def c_src(spec):
    loop = {
        "linear": "    for (long i = 1; i <= n; i++) {{ {step} }}",
        "triangular": "    for (long i = 1; i <= n; i++) for (long j = i; j <= n; j++) {{ {step} }}",
    }[spec.get("loop", "linear")].format(step=spec["c_step"])
    return (
        "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n#include <math.h>\n"
        + spec.get("c_helpers", "") + "\n"
        "int main(int argc, char *argv[]) {\n"
        "    if (argc < 2) return 1;\n"
        "    long n = atol(argv[1]);\n"
        "    long s = 0;\n"
        + loop + "\n"
        '    printf("%ld\\n", s);\n    return 0;\n}\n'
    )


def rust_src(spec):
    loop = {
        "linear": "    for i in 1..=n { STEP }",
        "triangular": "    for i in 1..=n { for j in i..=n { STEP } }",
    }[spec.get("loop", "linear")].replace("STEP", spec["rust_step"])
    return (
        "use std::env;\n" + spec.get("rust_helpers", "") + "\n"
        "fn main() {\n"
        "    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();\n"
        "    let mut s: i64 = 0;\n"
        + loop + "\n"
        '    println!("{}", s);\n}\n'
    )


def go_src(spec):
    imports = ["fmt", "os", "strconv"] + spec.get("go_imports", [])
    imp = "\n".join(f'\t"{i}"' for i in dict.fromkeys(imports))
    loop = {
        "linear": "\tfor i := int64(1); i <= n; i++ { STEP }",
        "triangular": "\tfor i := int64(1); i <= n; i++ { for j := i; j <= n; j++ { STEP } }",
    }[spec.get("loop", "linear")].replace("STEP", spec["go_step"])
    return (
        "package main\n\nimport (\n" + imp + "\n)\n"
        + spec.get("go_helpers", "") + "\n"
        "func main() {\n"
        "\tn64, _ := strconv.Atoi(os.Args[1])\n\tn := int64(n64)\n\tvar s int64 = 0\n"
        + loop + "\n"
        "\tfmt.Println(s)\n}\n"
    )


def py_src(spec):
    loop = {
        "linear": "for i in range(1, n + 1):\n    {step}",
        "triangular": "for i in range(1, n + 1):\n    for j in range(i, n + 1):\n        {step}",
    }[spec.get("loop", "linear")].format(step=spec["py_step"])
    return (
        "import sys\n" + spec.get("py_helpers", "") + "\n"
        "n = int(sys.argv[1])\ns = 0\n" + loop + "\nprint(s)\n"
    )


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def build_one(spec, verify=True):
    d = os.path.join(BENCH, spec["dir"])
    os.makedirs(d, exist_ok=True)
    # Author a counterpart ONLY for a language LOGOS must beat (the catalog `beat`
    # set) — including a language LOGOS cannot beat (e.g. C's native `x+y` for
    # operator.*) would wrongly fail the strictly-fastest gate. Python is ALWAYS
    # emitted: it is the `expected` oracle and LOGOS beats interpreted Python on
    # any compute loop.
    beat = set(spec.get("beat", []))
    files = {"main.lg": logos_src(spec), "main.py": py_src(spec)}
    if "c" in beat:
        files["main.c"] = c_src(spec)
    if "rust" in beat:
        files["main.rs"] = rust_src(spec)
    if "go" in beat:
        files["main.go"] = go_src(spec)
    for name, content in files.items():
        open(os.path.join(d, name), "w").write(content)

    arg = str(spec["arg"])
    # expected := python reference
    r = run(["python3", os.path.join(d, "main.py"), arg])
    if r.returncode != 0:
        return spec["dir"], False, f"python failed: {r.stderr.strip()[:200]}"
    expected = r.stdout.strip()

    meta = {
        "capability": spec["capability"],
        "workload": spec.get("workload", ""),
        "arg": arg, "expected": expected,
        "logos_uses_stdlib": True,
        "beat": spec.get("beat", []),
    }
    json.dump(meta, open(os.path.join(d, "meta.json"), "w"), indent=2)

    if not verify:
        return spec["dir"], True, f"expected={expected} (references unverified)"

    tmp = tempfile.mkdtemp()
    disagree = []
    try:
        if "c" in files:
            b = os.path.join(tmp, "c")
            if run(["gcc", "-O3", "-march=native", "-o", b, os.path.join(d, "main.c"), "-lm"]).returncode == 0:
                got = run([b, arg]).stdout.strip()
                if got != expected: disagree.append(f"c={got}")
            else: disagree.append("c:compile-error")
        if "rust" in files:
            b = os.path.join(tmp, "rs")
            if run(["rustc", "--edition", "2021", "-O", "-o", b, os.path.join(d, "main.rs")]).returncode == 0:
                got = run([b, arg]).stdout.strip()
                if got != expected: disagree.append(f"rust={got}")
            else: disagree.append("rust:compile-error")
        if "go" in files:
            b = os.path.join(tmp, "go")
            env = dict(os.environ, GOFLAGS="", GO111MODULE="off", GOCACHE=os.path.join(tmp, "gocache"))
            if run(["go", "build", "-o", b, os.path.join(d, "main.go")], env=env).returncode == 0:
                got = run([b, arg]).stdout.strip()
                if got != expected: disagree.append(f"go={got}")
            else: disagree.append("go:compile-error")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if disagree:
        return spec["dir"], False, f"expected={expected} DISAGREE: {', '.join(disagree)}"
    return spec["dir"], True, f"expected={expected} refs✓"


def main():
    specs = json.load(open(sys.argv[1]))
    if isinstance(specs, dict):
        specs = [specs]
    verify = "--no-verify" not in sys.argv
    ok = 0
    for spec in specs:
        d, good, msg = build_one(spec, verify=verify)
        print(f"  [{'OK ' if good else 'XX '}] {d:32} {msg}")
        ok += good
    print(f"\n{ok}/{len(specs)} packages authored + reference-verified")


if __name__ == "__main__":
    main()
