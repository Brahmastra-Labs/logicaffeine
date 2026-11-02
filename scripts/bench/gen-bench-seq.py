#!/usr/bin/env python3
"""Generate a stdlib-parity bench package for a SEQUENCE-workload capability.

Unlike the scalar `f(i)` fold (gen-bench-pkg.py), these ops act on a whole
`Seq of Int`. The skeleton (identical logic in all 5 languages) builds `xs` of
size M once, then runs `reps` (=argv[1]) iterations that each PERTURB one element
of `xs` (a cheap O(1) write that defeats loop-invariant hoisting — the
[[bench-c-hoists-pure-calls]] lesson) and fold the op's result into `checksum`.

LOGOS `Seq` is 1-INDEXED; the skeleton accounts for it (`item (idx+1) of xs`).
All languages build xs[k] = (k*2654435761) % 2^20 and perturb xs[r%M] = r % 2^20,
so the checksum is identical across languages. `expected` := the Python run;
the generator cross-checks that every `beat`-language reference agrees.

Per-cap spec:
{
  "dir": "iter_sum", "capability": "iter.sum", "beat": ["python","rust"],
  "size": 4096,                  # M (baked)
  "arg": "200000",              # reps (argv[1]); scales runtime
  "workload": "...",
  "needs_ys": false,            # build a second Seq `ys` too (binary ops)
  "body": {                      # fold-into-checksum body per language
    "logos": "Set checksum to checksum + sumSeq(xs).",
    "py":    "checksum += sum(xs)",
    "rust":  "checksum += xs.iter().sum::<i64>();",
    "go":    "for _, v := range xs { checksum += v }",
    "c":     "for (long k=0;k<M;k++) checksum += xs[k];"
  },
  "helpers": {"go_imports": [], "c_defs": "", "rust_helpers": "", "go_helpers": "", "py_helpers": ""}
}
Only the `beat` languages' bodies are required (plus "logos" and "py" always).
"""
import json, os, subprocess, sys, tempfile, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BENCH = os.path.join(ROOT, "benchmarks/parity/bench")
G = "2654435761"          # Knuth multiplicative hash constant
MOD = "1048576"            # 2^20 — keeps values (and i64 checksum) bounded


def _ys_logos(needs):
    return ("Let mutable ys be a new Seq of Int.\nLet mutable k2 be 0.\n"
            "While k2 is less than m:\n    Push (k2 * 1013904223 + 12345) % " + MOD + " to ys.\n"
            "    Set k2 to k2 + 1.\n") if needs else ""


def logos_src(s):
    h = s.get("helpers", {})
    return (
        "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n"
        + (h.get("logos_helpers", "") + "\n" if h.get("logos_helpers") else "")
        + "\n## Main\n"
        "Let arguments be args().\nLet reps be parseInt(item 2 of arguments).\n"
        f"Let m be {s['size']}.\n"
        "Let mutable xs be a new Seq of Int.\nLet mutable k be 0.\n"
        f"While k is less than m:\n    Push (k * {G}) % {MOD} to xs.\n    Set k to k + 1.\n"
        + _ys_logos(s.get("needs_ys"))
        + ("Set xs to sortSeq(xs).\n" if s.get("sorted") else "")
        + "Let mutable checksum be 0.\nLet mutable r be 0.\n"
        "While r is less than reps:\n"
        + ("" if s.get("sorted") else f"    Set item ((r % m) + 1) of xs to r % {MOD}.\n")
        + f"    {s['body']['logos']}\n"
        "    Set r to r + 1.\n"
        "Show checksum.\n"
    )


def py_src(s):
    h = s.get("helpers", {})
    ys = (f"ys = [(k * 1013904223 + 12345) % {MOD} for k in range(M)]\n") if s.get("needs_ys") else ""
    return (
        "import sys\n" + (h.get("py_helpers", "") + "\n" if h.get("py_helpers") else "")
        + f"reps = int(sys.argv[1])\nM = {s['size']}\n"
        f"xs = [(k * {G}) % {MOD} for k in range(M)]\n" + ys
        + ("xs.sort()\n" if s.get("sorted") else "")
        + "checksum = 0\n"
        "for r in range(reps):\n"
        + ("" if s.get("sorted") else f"    xs[r % M] = r % {MOD}\n")
        + f"    {s['body']['py']}\n"
        "print(checksum)\n"
    )


def rust_src(s):
    h = s.get("helpers", {})
    ys = (f"    let mut ys: Vec<i64> = (0..m).map(|k| ((k as i64) * 1013904223 + 12345) % {MOD}).collect();\n") if s.get("needs_ys") else ""
    return (
        "use std::env;\n" + (h.get("rust_helpers", "") + "\n" if h.get("rust_helpers") else "")
        + "fn main() {\n"
        "    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();\n"
        f"    let m: usize = {s['size']};\n"
        f"    let mut xs: Vec<i64> = (0..m).map(|k| ((k as i64) * {G}) % {MOD}).collect();\n" + ys
        + ("    xs.sort();\n" if s.get("sorted") else "")
        + "    let mut checksum: i64 = 0;\n"
        "    for r in 0..reps {\n"
        + ("" if s.get("sorted") else f"        xs[(r as usize) % m] = r % {MOD};\n")
        + f"        {s['body']['rust']}\n"
        "    }\n    println!(\"{}\", checksum);\n}\n"
    )


def go_src(s):
    h = s.get("helpers", {})
    imports = ["fmt", "os", "strconv"] + h.get("go_imports", []) + (["sort"] if s.get("sorted") else [])
    imp = "\n".join(f'\t"{i}"' for i in dict.fromkeys(imports))
    ys = (f"\tys := make([]int64, M)\n\tfor k := 0; k < M; k++ {{ ys[k] = (int64(k)*1013904223 + 12345) % {MOD} }}\n") if s.get("needs_ys") else ""
    return (
        "package main\nimport (\n" + imp + "\n)\n"
        + (h.get("go_helpers", "") + "\n" if h.get("go_helpers") else "")
        + "func main() {\n"
        "\treps, _ := strconv.Atoi(os.Args[1])\n"
        f"\tM := {s['size']}\n"
        f"\txs := make([]int64, M)\n\tfor k := 0; k < M; k++ {{ xs[k] = (int64(k) * {G}) % {MOD} }}\n" + ys
        + ("\tsort.Slice(xs, func(a, b int) bool { return xs[a] < xs[b] })\n" if s.get("sorted") else "")
        + "\tvar checksum int64 = 0\n"
        "\tfor r := 0; r < reps; r++ {\n"
        + ("" if s.get("sorted") else f"\t\txs[r % M] = int64(r) % {MOD}\n")
        + f"\t\t{s['body']['go']}\n"
        "\t}\n\tfmt.Println(checksum)\n}\n"
    )


def c_src(s):
    h = s.get("helpers", {})
    ys = (f"    long* ys = malloc(M * sizeof(long));\n    for (long k=0;k<M;k++) ys[k]=(k*1013904223L+12345L) % {MOD}L;\n") if s.get("needs_ys") else ""
    return (
        "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n"
        + (h.get("c_helpers", "") + "\n" if h.get("c_helpers") else "")
        + ("static int _cmpl(const void*a,const void*b){long x=*(const long*)a,y=*(const long*)b;return (x>y)-(x<y);}\n" if s.get("sorted") else "")
        + (h.get("c_defs", "") + "\n" if h.get("c_defs") else "")
        + "int main(int argc, char** argv){\n"
        "    long reps = atol(argv[1]);\n"
        f"    long M = {s['size']};\n"
        f"    long* xs = malloc(M * sizeof(long));\n    for (long k=0;k<M;k++) xs[k]=(k*{G}L) % {MOD}L;\n" + ys
        + ("    qsort(xs, M, sizeof(long), _cmpl);\n" if s.get("sorted") else "")
        + "    long checksum = 0;\n"
        "    for (long r=0;r<reps;r++){\n"
        + ("" if s.get("sorted") else f"        xs[r % M] = r % {MOD}L;\n")
        + f"        {s['body']['c']}\n"
        "    }\n    printf(\"%ld\\n\", checksum);\n    return 0;\n}\n"
    )


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def build_one(spec):
    d = os.path.join(BENCH, spec["dir"])
    os.makedirs(d, exist_ok=True)
    beat = set(spec.get("beat", []))
    files = {"main.lg": logos_src(spec), "main.py": py_src(spec)}
    if "c" in beat: files["main.c"] = c_src(spec)
    if "rust" in beat: files["main.rs"] = rust_src(spec)
    if "go" in beat: files["main.go"] = go_src(spec)
    for name, content in files.items():
        open(os.path.join(d, name), "w").write(content)
    arg = str(spec["arg"])
    r = run(["python3", os.path.join(d, "main.py"), arg])
    if r.returncode != 0:
        return spec["dir"], False, f"python failed: {r.stderr.strip()[:200]}"
    expected = r.stdout.strip()
    json.dump({"capability": spec["capability"], "workload": spec.get("workload", ""),
               "arg": arg, "expected": expected, "logos_uses_stdlib": True,
               "beat": spec.get("beat", [])},
              open(os.path.join(d, "meta.json"), "w"), indent=2)
    tmp = tempfile.mkdtemp(); disagree = []
    try:
        if "main.c" in files:
            b = os.path.join(tmp, "c")
            if run(["gcc", "-O3", "-march=native", "-o", b, os.path.join(d, "main.c"), "-lm"]).returncode == 0:
                got = run([b, arg]).stdout.strip()
                if got != expected: disagree.append(f"c={got}")
            else: disagree.append("c:compile-error")
        if "main.rs" in files:
            b = os.path.join(tmp, "rs")
            if run(["rustc", "--edition", "2021", "-O", "-o", b, os.path.join(d, "main.rs")]).returncode == 0:
                got = run([b, arg]).stdout.strip()
                if got != expected: disagree.append(f"rust={got}")
            else: disagree.append("rust:compile-error")
        if "main.go" in files:
            b = os.path.join(tmp, "go")
            env = dict(os.environ, GO111MODULE="off", GOCACHE=os.path.join(tmp, "gc"))
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
    if isinstance(specs, dict): specs = [specs]
    ok = 0
    for spec in specs:
        dn, good, msg = build_one(spec)
        print(f"  [{'OK ' if good else 'XX '}] {dn:28} {msg}")
        ok += good
    print(f"\n{ok}/{len(specs)} sequence packages authored + reference-verified")


if __name__ == "__main__":
    main()
