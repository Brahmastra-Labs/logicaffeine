#!/usr/bin/env python3
"""Generate a stdlib-parity bench package for a TEXT capability (operates on a String).

Strings are immutable, so we can't perturb in place. Anti-hoist trick: each iteration
`s` is rebuilt as `("a" * (r%100)) + base` — a VARYING-LENGTH string, so even a
length-preserving op (upper/lower/reverse) yields a data-dependent result that can't be
hoisted. `base` (length M, pattern "abcABC012 ") is built once; an optional fixed `sub`
needle supports search ops. Per-cap `body` folds the op result into an i64 `checksum`
(via `length of` / an index / a bool count — LOGOS has no ord()). All languages build the
identical base + s, so checksums match. `expected` := Python; the generator cross-checks the
`beat`-language references.
"""
import json, os, subprocess, sys, tempfile, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BENCH = os.path.join(ROOT, "benchmarks/parity/bench")
PAT = "abcABC012 "   # 10 chars: lower, upper, digits, space


def logos_src(s):
    sub = (f'Let sub be "{s["sub"]}".\n' if s.get("sub") else "")
    return (
        "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\n"
        "Let arguments be args().\nLet reps be parseInt(item 2 of arguments).\n"
        f"Let cells be {s['size'] // 10}.\n"
        f'Let base be repeatText("{PAT}", cells).\n' + sub
        + "Let mutable checksum be 0.\nLet mutable r be 0.\n"
        "While r is less than reps:\n"
        '    Let s be repeatText("a", r % 100) + base.\n'
        f"    {s['body']['logos']}\n"
        "    Set r to r + 1.\n"
        "Show checksum.\n"
    )


def py_src(s):
    h = s.get("helpers", {})
    sub = (f'sub = "{s["sub"]}"\n' if s.get("sub") else "")
    return (
        "import sys\n" + (h.get("py_helpers", "") + "\n" if h.get("py_helpers") else "")
        + f"reps = int(sys.argv[1])\n"
        f'base = "{PAT}" * {s["size"] // 10}\n' + sub
        + "checksum = 0\n"
        "for r in range(reps):\n"
        '    s = "a" * (r % 100) + base\n'
        f"    {s['body']['py']}\n"
        "print(checksum)\n"
    )


def rust_src(s):
    h = s.get("helpers", {})
    sub = (f'    let sub = "{s["sub"]}";\n' if s.get("sub") else "")
    return (
        "use std::env;\n" + (h.get("rust_helpers", "") + "\n" if h.get("rust_helpers") else "")
        + "fn main() {\n"
        "    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();\n"
        f'    let base = "{PAT}".repeat({s["size"] // 10});\n' + sub
        + "    let mut checksum: i64 = 0;\n"
        "    for r in 0..reps {\n"
        '        let s = "a".repeat((r as usize) % 100) + &base;\n'
        f"        {s['body']['rust']}\n"
        "    }\n    println!(\"{}\", checksum);\n}\n"
    )


def go_src(s):
    h = s.get("helpers", {})
    imports = ["fmt", "os", "strconv", "strings"] + h.get("go_imports", [])
    imp = "\n".join(f'\t"{i}"' for i in dict.fromkeys(imports))
    sub = (f'\tsub := "{s["sub"]}"\n' if s.get("sub") else "")
    return (
        "package main\nimport (\n" + imp + "\n)\n"
        + (h.get("go_helpers", "") + "\n" if h.get("go_helpers") else "")
        + "func main() {\n"
        "\treps, _ := strconv.Atoi(os.Args[1])\n"
        f'\tbase := strings.Repeat("{PAT}", {s["size"] // 10})\n' + sub
        + "\tvar checksum int64 = 0\n"
        "\tfor r := 0; r < reps; r++ {\n"
        '\t\ts := strings.Repeat("a", r%100) + base\n'
        f"\t\t{s['body']['go']}\n"
        "\t}\n\tfmt.Println(checksum)\n}\n"
    )


def c_src(s):
    # C rarely a beat target for text (Python's C-backed str dominates); supported for length/is_* via a manual buffer.
    h = s.get("helpers", {})
    return (
        "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n"
        + (h.get("c_helpers", "") + "\n" if h.get("c_helpers") else "")
        + "int main(int argc, char** argv){\n"
        "    long reps = atol(argv[1]);\n"
        f"    long cells = {s['size'] // 10};\n"
        f'    long blen = cells * 10;\n    char* base = malloc(blen + 200);\n'
        f'    for (long c = 0; c < cells; c++) memcpy(base + c*10, "{PAT}", 10);\n    base[blen] = 0;\n'
        + (f'    char* sub = "{s["sub"]}";\n' if s.get("sub") else "")
        + "    char* s = malloc(blen + 200);\n    long checksum = 0;\n"
        "    for (long r = 0; r < reps; r++) {\n"
        "        long pre = r % 100;\n        memset(s, 'a', pre);\n        memcpy(s + pre, base, blen + 1);\n"
        f"        {s['body']['c']}\n"
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
               "beat": spec.get("beat", [])}, open(os.path.join(d, "meta.json"), "w"), indent=2)
    tmp = tempfile.mkdtemp(); disagree = []
    try:
        if "main.c" in files:
            b = os.path.join(tmp, "c")
            if run(["gcc", "-O3", "-march=native", "-o", b, os.path.join(d, "main.c")]).returncode == 0:
                if run([b, arg]).stdout.strip() != expected: disagree.append("c")
            else: disagree.append("c:compile-error")
        if "main.rs" in files:
            b = os.path.join(tmp, "rs")
            if run(["rustc", "--edition", "2021", "-O", "-o", b, os.path.join(d, "main.rs")]).returncode == 0:
                if run([b, arg]).stdout.strip() != expected: disagree.append("rust")
            else: disagree.append("rust:compile-error")
        if "main.go" in files:
            b = os.path.join(tmp, "go")
            env = dict(os.environ, GO111MODULE="off", GOCACHE=os.path.join(tmp, "gc"))
            if run(["go", "build", "-o", b, os.path.join(d, "main.go")], env=env).returncode == 0:
                if run([b, arg]).stdout.strip() != expected: disagree.append("go")
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
    print(f"\n{ok}/{len(specs)} text packages authored + reference-verified")


if __name__ == "__main__":
    main()
