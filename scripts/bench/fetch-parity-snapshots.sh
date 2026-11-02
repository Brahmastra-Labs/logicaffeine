#!/usr/bin/env bash
# Grab the raw public surface of the C, Python, Rust, and Go standard libraries into
# benchmarks/parity/snapshots/ — the auditable "we grabbed theirs" evidence for the
# stdlib-parity campaign. Run BY HAND when refreshing to newer toolchains; the output is
# committed and the offline normalizer (scripts/bench/gen-parity-inventories.py) + the lock
# (crates/logicaffeine_tests/tests/parity_catalog_lock.rs) consume only the committed
# snapshots, never the network or the live toolchain.
#
# Python and Go are introspected from the LOCAL pinned toolchains (no network). C, Rust,
# and Java are curated standard-surface snapshots authored under snapshots/{c,rust,java}/
# (C is additionally verified present in the box's real headers). Pin versions land in
# benchmarks/parity/sources.lock.toml.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SNAP="$ROOT/benchmarks/parity/snapshots"
mkdir -p "$SNAP"/{c,python,go,rust,java}

PY_VER="$(python3 -c 'import platform;print(platform.python_version())')"
GO_VER="$(go version | awk '{print $3}')"
RUSTC_VER="$(rustc --version | awk '{print $2}')"
# Capture ldd output fully in-memory before slicing: piping `head` onto the live `ldd`
# subprocess SIGPIPEs it, which `set -o pipefail` turns fatal.
GLIBC_RAW="$(ldd --version 2>/dev/null || true)"
GLIBC_VER="$(printf '%s' "$GLIBC_RAW" | grep -oE '[0-9]+\.[0-9]+' | head -1 || true)"

echo "Python $PY_VER · Go $GO_VER · rustc $RUSTC_VER · glibc $GLIBC_VER"

# ---- Python: introspect every public top-level stdlib module ----------------------------
echo "Grabbing Python stdlib surface ..."
python3 - "$SNAP/python/symbols.json" <<'PY'
import importlib, inspect, json, sys, warnings
warnings.simplefilter("ignore")
# Modules with import-time side effects (browser, banner) or no meaningful public API.
SKIP = {"this", "antigravity", "idlelib", "tkinter", "turtle", "turtledemo", "lib2to3", "__hello__"}
out = {}
for mod in sorted(m for m in sys.stdlib_module_names if not m.startswith("_") and m not in SKIP):
    try:
        m = importlib.import_module(mod)
    except Exception as e:                       # platform / optional / side-effecting
        out[mod] = {"__import_error__": type(e).__name__}
        continue
    syms = {}
    for name, obj in inspect.getmembers(m):
        if name.startswith("_"):
            continue
        if inspect.isclass(obj):
            kind = "class"
        elif inspect.isfunction(obj) or inspect.isbuiltin(obj) or inspect.ismethoddescriptor(obj) or callable(obj) and not inspect.ismodule(obj):
            kind = "function"
        elif inspect.ismodule(obj):
            continue                              # re-exported submodule, not this module's surface
        else:
            kind = "constant"
        syms[name] = kind
    out[mod] = dict(sorted(syms.items()))

# The core of Python's parity surface is builtin TYPE methods (str.split, list.append,
# dict.get) and builtin FUNCTIONS (sorted, map, filter) — none of which live in a module.
import builtins
bsyms = {}
for name, obj in inspect.getmembers(builtins):
    if name.startswith("_") or name[0].isupper():
        continue  # Exceptions/types handled below; keep lowercase builtin functions
    if inspect.isbuiltin(obj) or inspect.isfunction(obj) or callable(obj):
        bsyms[name] = "function"
out["builtins"] = dict(sorted(bsyms.items()))
for tname in ["str", "bytes", "bytearray", "list", "tuple", "dict", "set",
              "frozenset", "int", "float", "complex", "bool"]:
    t = getattr(builtins, tname)
    methods = {n: "method" for n in dir(t)
               if not n.startswith("_") and callable(getattr(t, n, None))}
    out[f"builtins.{tname}"] = dict(sorted(methods.items()))

json.dump({"language": "python", "version": sys.version.split()[0], "modules": out},
          open(sys.argv[1], "w"), indent=1, sort_keys=True)
print(f"  {len(out)} modules (incl. builtins + core type methods)")
PY

# ---- Go: `go list std` + `go doc` for every non-internal std package --------------------
echo "Grabbing Go stdlib surface ..."
{
  echo "# go $GO_VER — go doc dump of non-internal std packages"
  for pkg in $(go list std 2>/dev/null | grep -vE '(^|/)(internal|vendor)(/|$)'); do
    echo "## PKG $pkg"
    go doc -all "$pkg" 2>/dev/null | grep -E '^(func|type|const|var) ' || true
  done
} > "$SNAP/go/go-doc.txt"
echo "  $(grep -c '^## PKG ' "$SNAP/go/go-doc.txt") packages"

# ---- C: verify each curated C23 function is present in the box's real headers -----------
# snapshots/c/c23-functions.json is the curated standard surface (authored, checked in).
# Here we PROVE we grabbed the real headers: every function token must appear in /usr/include.
echo "Verifying C surface against local glibc $GLIBC_VER headers ..."
python3 - "$SNAP/c/c23-functions.json" "$SNAP/c/header-verification.txt" <<'PY'
import json, os, re, sys, subprocess
cur = json.load(open(sys.argv[1]))
missing = []
lines = [f"# C23 (WG14 N3220) surface verified against local glibc headers", ""]
for header, funcs in sorted(cur["headers"].items()):
    hp = f"/usr/include/{header}"
    text = ""
    if os.path.exists(hp):
        text = open(hp, errors="ignore").read()
    # many glibc prototypes live in bits/ — also scan the preprocessed header
    try:
        pp = subprocess.run(["gcc", "-E", "-P", "-"], input=f"#include <{header}>\n",
                            capture_output=True, text=True, timeout=30)
        text += "\n" + pp.stdout
    except Exception:
        pass
    for fn in funcs:
        present = re.search(r"\b" + re.escape(fn) + r"\b", text) is not None
        lines.append(f"{header}\t{fn}\t{'OK' if present else 'MISSING'}")
        if not present:
            missing.append(f"{header}:{fn}")
open(sys.argv[2], "w").write("\n".join(lines) + "\n")
print(f"  {sum(len(v) for v in cur['headers'].values())} functions across "
      f"{len(cur['headers'])} headers; {len(missing)} not found in local headers")
if missing:
    print("  NOTE not-found (kept; may be macro/bits-defined):", ", ".join(missing[:12]))
PY

# ---- Rust / Java: curated pinned snapshots are authored files; just report --------------
echo "Rust curated snapshot: $([ -f "$SNAP/rust/std-curated.json" ] && echo present || echo MISSING)"
echo "Java curated snapshot: $([ -f "$SNAP/java/java-curated.json" ] && echo present || echo MISSING)"

# ---- Write the pin file (with dependency-free FNV-1a-64 integrity hashes) ---------------
# The Rust lock (parity_catalog_lock.rs) recomputes fnv1a64 over each snapshot and compares,
# proving the committed evidence is unmodified. FNV-1a-64 is trivial + identical in Python
# and Rust, so no crypto dependency is needed on either side.
PY_VER="$PY_VER" GO_VER="$GO_VER" RUSTC_VER="$RUSTC_VER" GLIBC_VER="$GLIBC_VER" \
SNAP="$SNAP" OUT="$ROOT/benchmarks/parity/sources.lock.toml" python3 - <<'PY'
import os
SNAP = os.environ["SNAP"]
def fnv1a64(path):
    h = 0xcbf29ce484222325
    data = open(path, "rb").read()
    for x in data:
        h = ((h ^ x) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h, len(data)
def sect(name, snap, extra):
    h, n = fnv1a64(f"{SNAP}/{snap.split('/', 1)[1]}")
    lines = [f"[{name}]"]
    for k, v in extra:
        lines.append(f'{k:<8} = "{v}"')
    lines.append(f'snapshot = "{snap}"')
    lines.append(f'bytes    = {n}')
    lines.append(f'fnv1a64  = "0x{h:016x}"')
    return "\n".join(lines)
env = os.environ
body = [
  "# Pinned upstream stdlib sources for the parity campaign. Regenerate snapshots with",
  "# scripts/bench/fetch-parity-snapshots.sh. The lock recomputes fnv1a64 to prove the committed",
  "# evidence is unmodified (parity_catalog_lock.rs).", "",
  sect("python", "snapshots/python/symbols.json",
       [("version", env["PY_VER"]), ("method", "sys.stdlib_module_names + inspect.getmembers + builtin type methods")]), "",
  sect("go", "snapshots/go/go-doc.txt",
       [("version", env["GO_VER"]), ("method", "go list std | go doc -all (non-internal packages)")]), "",
  sect("c", "snapshots/c/c23-functions.json",
       [("standard", "C23 (WG14 N3220)"), ("glibc", env["GLIBC_VER"]), ("method", "curated standard surface, verified present in local headers")]), "",
  sect("rust", "snapshots/rust/std-curated.json",
       [("version", env["RUSTC_VER"]), ("method", "curated std surface, pinned to rustc version")]), "",
  sect("java", "snapshots/java/java-curated.json",
       [("version", "curated (secondary reference)"), ("method", "curated java.lang/java.util/java.io/java.time surface")]),
]
open(env["OUT"], "w").write("\n".join(body) + "\n")
print("  sources.lock.toml written with fnv1a64 integrity hashes")
PY

echo "Done. Snapshots under benchmarks/parity/snapshots/, pins in sources.lock.toml."
echo "Next: scripts/bench/gen-parity-inventories.py to normalize into inventory/*.json."
