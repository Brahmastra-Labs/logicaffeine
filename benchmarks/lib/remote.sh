#!/usr/bin/env bash
# The bench box's half of benchmarks/run-remote.sh. run-remote.sh copies this tree to <root>/repo on the machine and
# runs this script there; <root> is the tree's parent directory, which also holds the toolchains (tools/) and one
# directory per run (runs/<id>/).
#
#   remote.sh prepare       install the toolchains the published run measured with: rustc, Node and clang at the
#                           versions results/latest.json records, beside the system's own (never replacing them),
#                           every other toolchain through setup-local.sh, and the login's user manager from boot
#                           (lingering), which run.sh's memory-capped scope runs in
#   remote.sh start         start run.sh detached and print the run's id; refuses while another run is going
#   remote.sh status [id]   the run's status lines and the tail of its output (the newest run by default)
#   remote.sh path [id]     the run's directory
#
# A run quiets the machine for its own duration: every timed CPU on the performance governor, sleep masked until the
# next boot at most, the package updaters paused. When run.sh exits, pass or fail, the machine gets each one back.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
ROOT="$(cd "$REPO/.." && pwd)"
RUNS="$ROOT/runs"
TOOLS="$ROOT/tools"
SYS="${BENCH_SYS_ROOT:-/sys}"
PROC="${BENCH_PROC_ROOT:-/proc}"
SUDO=sudo
[ "$(id -u)" -eq 0 ] && SUDO=""
SLEEP_TARGETS=(sleep.target suspend.target hibernate.target hybrid-sleep.target)
UPDATE_TIMERS=(apt-daily.timer apt-daily-upgrade.timer)

stamp() { date -u +%FT%TZ; }
say() { printf '[remote] %s\n' "$*"; }

# The version results/latest.json recorded for a row of lib/languages.tsv.
reference_version() {
    python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["metadata"]["versions"].get(sys.argv[2], ""))' \
        "$REPO/benchmarks/results/latest.json" "$1"
}
pinned_rust() { reference_version rust | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1; }
pinned_node() { reference_version js | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' | head -1; }
pinned_clang() { reference_version c_clang | grep -oE 'version [0-9]+' | grep -oE '[0-9]+' | head -1; }

# run.sh holds a run in a memory-capped `systemd-run --user` scope, which needs the login's user manager. A login
# through an sshd without PAM is never told where that manager listens; it is always at the login's runtime directory.
user_bus() {
    export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
    export DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=$XDG_RUNTIME_DIR/bus}"
}

# The PATH a run measures with: the pinned toolchains ahead of the system's.
run_path() { printf '%s' "$TOOLS/bin:$TOOLS/node/bin:$TOOLS/cargo/bin:$PATH"; }

install_rust() {
    local v="$1"
    export CARGO_HOME="$TOOLS/cargo" RUSTUP_HOME="$TOOLS/rustup"
    if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
        curl -fsSL https://sh.rustup.rs -o "$TOOLS/rustup-init.sh"
        sh "$TOOLS/rustup-init.sh" -y --no-modify-path --profile minimal --default-toolchain none
    fi
    "$CARGO_HOME/bin/rustup" toolchain install "$v" --profile minimal
    "$CARGO_HOME/bin/rustup" default "$v"
}

install_node() {
    local v="$1" base f
    if [ "$("$TOOLS/node/bin/node" --version 2>/dev/null)" = "$v" ]; then return; fi
    base="https://nodejs.org/dist/$v"
    f="node-$v-linux-x64.tar.xz"
    curl -fsSL "$base/$f" -o "$TOOLS/$f"
    curl -fsSL "$base/SHASUMS256.txt" | grep " $f\$" | (cd "$TOOLS" && sha256sum -c -)
    rm -rf "$TOOLS/node"
    mkdir -p "$TOOLS/node"
    tar -xJf "$TOOLS/$f" -C "$TOOLS/node" --strip-components=1
    rm -f "$TOOLS/$f"
}

install_clang() {
    local major="$1" t
    if ! command -v "clang-$major" >/dev/null; then
        curl -fsSL https://apt.llvm.org/llvm.sh -o "$TOOLS/llvm.sh"
        $SUDO bash "$TOOLS/llvm.sh" "$major"
    fi
    $SUDO apt-get install -y -q "llvm-$major-linker-tools" "lld-$major"
    for t in clang clang++ llvm-ar llvm-nm llvm-ranlib ld.lld; do
        ln -sfn "$(command -v "$t-$major")" "$TOOLS/bin/$t"
    done
}

prepare() {
    mkdir -p "$TOOLS/bin" "$RUNS" "$ROOT/tmp"
    local rust node clang
    rust=$(pinned_rust) node=$(pinned_node) clang=$(pinned_clang)
    [ -n "$rust" ] && [ -n "$node" ] && [ -n "$clang" ] || {
        echo "results/latest.json records no rust/js/c_clang version to pin (rust='$rust' js='$node' c_clang='$clang')" >&2
        exit 1
    }
    say "pinning rustc $rust, Node $node, clang $clang (the published run's toolchains)"
    $SUDO apt-get update -q
    $SUDO apt-get install -y -q build-essential pkg-config libssl-dev time rsync curl wget xz-utils ca-certificates \
        gnupg lsb-release software-properties-common python3 jq bc
    install_clang "$clang"
    install_node "$node"
    install_rust "$rust"
    PATH="$(run_path)" bash "$REPO/benchmarks/setup-local.sh"
    $SUDO loginctl enable-linger "$(id -un)"
    say "prepared $ROOT"
}

# The live run's id, if one is going.
active() {
    local pidfile pid
    for pidfile in "$RUNS"/*/pid; do
        [ -f "$pidfile" ] || continue
        pid=$(cat "$pidfile")
        if kill -0 "$pid" 2>/dev/null; then basename "$(dirname "$pidfile")"; return 0; fi
    done
    return 1
}

newest() { ls -1 "$RUNS" 2>/dev/null | sort | tail -1; }

run_dir() {
    local id="${1:-$(newest)}"
    [ -n "$id" ] && [ -d "$RUNS/$id" ] || { echo "no run '${1:-}' under $RUNS" >&2; exit 1; }
    printf '%s' "$RUNS/$id"
}

# The run's memory cap: three quarters of the machine, so a runaway program is killed inside the run's cgroup long
# before the machine itself is in trouble.
memory_cap() {
    local kb
    kb=$(awk '/^MemTotal:/ {print $2}' "$PROC/meminfo")
    printf '%dG' $(( kb * 3 / 4 / 1024 / 1024 ))
}

# quiet <dir>: put the machine in measuring order, writing to <dir>/restore exactly what to undo.
quiet() {
    local restore="$1/restore" f
    : > "$restore"
    for f in "$SYS"/devices/system/cpu/cpu[0-9]*/cpufreq/scaling_governor; do
        [ -f "$f" ] || continue
        printf 'governor %s %s\n' "$f" "$(cat "$f")" >> "$restore"
        echo performance | $SUDO tee "$f" >/dev/null
    done
    if command -v systemctl >/dev/null; then
        $SUDO systemctl mask --runtime "${SLEEP_TARGETS[@]}" >/dev/null 2>&1 && echo "sleep masked" >> "$restore"
        local t
        for t in "${UPDATE_TIMERS[@]}"; do
            if systemctl is-active --quiet "$t"; then
                $SUDO systemctl stop "$t" && echo "timer $t" >> "$restore"
            fi
        done
    fi
    if command -v snap >/dev/null && $SUDO snap refresh --hold >/dev/null 2>&1; then
        echo "snap held" >> "$restore"
    fi
}

# give_back <dir>: undo everything quiet recorded.
give_back() {
    local restore="$1/restore" kind a b
    [ -f "$restore" ] || return 0
    while read -r kind a b; do
        case "$kind" in
            governor) echo "$b" | $SUDO tee "$a" >/dev/null || true ;;
            sleep) $SUDO systemctl unmask --runtime "${SLEEP_TARGETS[@]}" >/dev/null 2>&1 || true ;;
            timer) $SUDO systemctl start "$a" || true ;;
            snap) $SUDO snap refresh --unhold >/dev/null 2>&1 || true ;;
        esac
    done < "$restore"
    mv "$restore" "$1/restored"
}

start() {
    local going id dir
    if going=$(active); then
        echo "a run is already going on this machine: $going" >&2
        exit 1
    fi
    id=$(date -u +%Y%m%dT%H%M%SZ)
    dir="$RUNS/$id"
    mkdir -p "$dir/logs" "$ROOT/tmp"
    echo "queued $(stamp)" > "$dir/status"
    setsid nohup bash "$HERE/remote.sh" _run "$id" > "$dir/runner.out" 2>&1 < /dev/null &
    echo $! > "$dir/pid"
    echo "$id"
}

_run() {
    local id="$1" dir="$RUNS/$1" rc=0
    quiet "$dir"
    trap "give_back $(printf '%q' "$dir")" EXIT
    echo "started $(stamp)" >> "$dir/status"
    user_bus
    (
        cd "$REPO"
        PATH="$(run_path)" RUSTUP_HOME="$TOOLS/rustup" CARGO_HOME="$TOOLS/cargo" RUSTUP_TOOLCHAIN="$(pinned_rust)" \
            TMPDIR="$ROOT/tmp" BENCH_RESULTS_DIR="$dir" BENCH_LOG_DIR="$dir/logs" \
            BENCH_MEMORY_MAX="${BENCH_MEMORY_MAX:-$(memory_cap)}" \
            bash benchmarks/run.sh > "$dir/run.out" 2>&1
    ) || rc=$?
    echo "finished $(stamp) exit=$rc" >> "$dir/status"
}

status() {
    local dir
    dir=$(run_dir "${1:-}")
    echo "run $(basename "$dir")"
    cat "$dir/status"
    if [ -f "$dir/run.out" ]; then
        echo "--- run.out"
        tail -n 5 "$dir/run.out"
    fi
}

case "${1:-}" in
    prepare) prepare ;;
    start) start ;;
    status) status "${2:-}" ;;
    path) run_dir "${2:-}"; echo ;;
    _run) _run "$2" ;;
    *) sed -n '2,15p' "$0" >&2; exit 2 ;;
esac
