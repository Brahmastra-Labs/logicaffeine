#!/usr/bin/env bash
# Builds an Arena rival exactly as the Arena does, so its instruction counts on this machine calibrate ours:
# its checker yaml's `build` script runs verbatim, with /bin/sh as lka.py runs it, in the Arena's own layout
# (<arena>/_build/checkers/<name>/src), with the Arena's rustc and llvm-profdata. The source is the GitHub
# tarball of the yaml's `rev`, never a clone. The yaml's `lka.py build-test init-prelude` regenerates the
# PGO training input from the Arena tree; the lean4export it needs is pre-placed from
# build-arena-inputs.py's work directory, so nothing is cloned.
#   [ARENA=<extracted lean-kernel-arena tree>] [EXPORTERS=<build-arena-inputs.py --work>/lean4export]
#   [TOOLCHAIN=1.95.0] scripts/lean/arena-crown/rivals.sh <checker-name>
# Prints the command that runs the rival on "$IN" the way the Arena's `run` does (in its source
# directory), to count with: instr.sh <input> --run "<that command>".
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
NAME="$1"
ARENA="${ARENA:?ARENA=<an extracted lean-kernel-arena tree at the pinned revision>}"
EXPORTERS="${EXPORTERS:-/tmp/arena-v434/work/lean4export}"
TOOLCHAIN="${TOOLCHAIN:-1.95.0}"
case "$(realpath "$ARENA")/" in "$ROOT"/*) echo "rivals: ARENA must lie outside this repository" >&2; exit 2 ;; esac
YAML="$ARENA/checkers/$NAME.yaml"
[ -f "$YAML" ] || { echo "rivals: no checker $YAML" >&2; exit 2; }
field() { python3 -c 'import sys, yaml; print(yaml.safe_load(open(sys.argv[1]))[sys.argv[2]], end="")' "$YAML" "$1"; }
URL="$(field url)"; REV="$(field rev)"
SRC="$ARENA/_build/checkers/$NAME/src"
LOGDIR="$ROOT/logs/arena-crown/rivals"; mkdir -p "$LOGDIR"
LOG="$LOGDIR/$NAME-${REV:0:8}.log"

VENV="$ARENA/../lka-venv"
[ -x "$VENV/bin/python3" ] || { python3 -m venv "$VENV" && "$VENV/bin/pip" install --quiet jinja2 jsonschema markdown pyyaml; } >&2
SHIMS="$ARENA/../rival-shims"; mkdir -p "$SHIMS"
ln -sfn "$(rustc +"$TOOLCHAIN" --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata" "$SHIMS/llvm-profdata"
mkdir -p "$ARENA/_build/lean4export"
for built in "$EXPORTERS"/*; do
  [ -x "$built/.lake/build/bin/lean4export" ] && ln -sfn "$built" "$ARENA/_build/lean4export/$(basename "$built")"
done

rm -rf "$SRC"; mkdir -p "$SRC"
OWNER_REPO="$(printf '%s' "$URL" | sed -E 's#^https://github.com/##; s#\.git$##; s#/$##')"
curl -sfL "https://codeload.github.com/$OWNER_REPO/tar.gz/$REV" | tar xz -C "$SRC" --strip-components=1
field build > "$ARENA/_build/checkers/$NAME/build.sh"
(
  cd "$SRC"
  env -u RUSTFLAGS -u CARGO_TARGET_DIR -u CARGO_BUILD_RUSTFLAGS \
    PATH="$SHIMS:$VENV/bin:$HOME/.elan/bin:$PATH" RUSTUP_TOOLCHAIN="$TOOLCHAIN" \
    /bin/sh "../build.sh"
) > "$LOG" 2>&1 || { echo "rivals: $NAME build failed; see $LOG" >&2; exit 1; }
printf 'cd %q && %s\n' "$SRC" "$(field run | head -1)"
