#!/usr/bin/env python3
"""Generate a stdlib-parity bench package for a BYTES capability (Seq of Int, 0-255).

Same amortize+perturb skeleton as gen-bench-seq.py, but the COMPETITOR languages use
their native BYTE types + byte stdlib so the comparison is fair: Go on `[]byte` with the
real `bytes` package (SIMD `bytes.Equal`/`Index`/`Count`), C on `unsigned char*` with
`memcmp`/`memchr` (`string.h`), Python on `bytearray`. LOGOS stays `Seq of Int`. All build
xs[k]=(k*2654435761)%256 and perturb xs[r%M]=r%256, so checksums match across languages.

`needs_ys` builds a second byte Seq `ys`. `sub_len` bakes a fixed-length needle `sub`
(the first `sub_len` bytes of xs at offset (r%(M-sub_len))) into every language for the
search ops (bytesIndexSub/HasPrefix/...) — a varying needle = anti-hoist without perturbing
data twice. Per-cap `body` folds the op result into `checksum`.
"""
import json, os, subprocess, sys, tempfile, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BENCH = os.path.join(ROOT, "benchmarks/parity/bench")
G = "2654435761"
MOD = "256"


def logos_src(s):
    ys = ("Let mutable ys be a new Seq of Int.\nLet mutable k2 be 0.\n"
          "While k2 is less than m:\n    Push (k2 * 1013904223 + 7) % 256 to ys.\n    Set k2 to k2 + 1.\n") if s.get("needs_ys") else ""
    return (
        "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\n"
        "Let arguments be args().\nLet reps be parseInt(item 2 of arguments).\n"
        f"Let m be {s['size']}.\n"
        "Let mutable xs be a new Seq of Int.\nLet mutable k be 0.\n"
        f"While k is less than m:\n    Push (k * {G}) % {MOD} to xs.\n    Set k to k + 1.\n" + ys
        + ("".join(f"Let mutable sub be a new Seq of Int.\n" if i == 0 else "" for i in range(1)) +
           "".join(f"Push {b} to sub.\n" for b in s["sub"]) if s.get("sub") else "")
        + "Let mutable checksum be 0.\nLet mutable r be 0.\n"
        "While r is less than reps:\n"
        f"    Set item ((r % m) + 1) of xs to r % {MOD}.\n"
        f"    {s['body']['logos']}\n"
        "    Set r to r + 1.\n"
        "Show checksum.\n"
    )


def py_src(s):
    h = s.get("helpers", {})
    ys = (f"ys = bytearray((k * 1013904223 + 7) % 256 for k in range(M))\n") if s.get("needs_ys") else ""
    return (
        "import sys\n" + (h.get("py_helpers", "") + "\n" if h.get("py_helpers") else "")
        + f"reps = int(sys.argv[1])\nM = {s['size']}\n"
        f"xs = bytearray((k * {G}) % 256 for k in range(M))\n" + ys
        + (f"sub = bytes({s['sub']})\n" if s.get("sub") else "")
        + "checksum = 0\n"
        "for r in range(reps):\n"
        f"    xs[r % M] = r % 256\n"
        f"    {s['body']['py']}\n"
        "print(checksum)\n"
    )


def go_src(s):
    h = s.get("helpers", {})
    imports = ["fmt", "os", "strconv"] + h.get("go_imports", [])
    imp = "\n".join(f'\t"{i}"' for i in dict.fromkeys(imports))
    ys = (f"\tys := make([]byte, M)\n\tfor k := 0; k < M; k++ {{ ys[k] = byte((k*1013904223 + 7) % 256) }}\n") if s.get("needs_ys") else ""
    return (
        "package main\nimport (\n" + imp + "\n)\n"
        + (h.get("go_helpers", "") + "\n" if h.get("go_helpers") else "")
        + "func main() {\n"
        "\treps, _ := strconv.Atoi(os.Args[1])\n"
        f"\tM := {s['size']}\n"
        f"\txs := make([]byte, M)\n\tfor k := 0; k < M; k++ {{ xs[k] = byte((k * {G}) % 256) }}\n" + ys
        + (f"\tsub := []byte{{{', '.join(str(b) for b in s['sub'])}}}\n" if s.get("sub") else "")
        + "\tvar checksum int64 = 0\n"
        "\tfor r := 0; r < reps; r++ {\n"
        "\t\txs[r % M] = byte(r % 256)\n"
        f"\t\t{s['body']['go']}\n"
        "\t}\n\tfmt.Println(checksum)\n}\n"
    )


def c_src(s):
    h = s.get("helpers", {})
    ys = (f"    unsigned char* ys = malloc(M);\n    for (long k=0;k<M;k++) ys[k]=(unsigned char)((k*1013904223L+7L) % 256);\n") if s.get("needs_ys") else ""
    return (
        "#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n"
        + (h.get("c_helpers", "") + "\n" if h.get("c_helpers") else "")
        + "int main(int argc, char** argv){\n"
        "    long reps = atol(argv[1]);\n"
        f"    long M = {s['size']};\n"
        f"    unsigned char* xs = malloc(M);\n    for (long k=0;k<M;k++) xs[k]=(unsigned char)((k*{G}L) % 256);\n" + ys
        + "    long checksum = 0;\n"
        "    for (long r=0;r<reps;r++){\n"
        "        xs[r % M] = (unsigned char)(r % 256);\n"
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
    print(f"\n{ok}/{len(specs)} bytes packages authored + reference-verified")


if __name__ == "__main__":
    main()
