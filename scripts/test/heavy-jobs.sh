#!/usr/bin/env bash
# Name every heavy non-cargo job running on this box: `heavy-jobs.sh` prints one line per job and exits 1 when
# there is any, 0 when there is none. Both full runners call it before their first cargo command and refuse to
# start beside a job it names (`heavy_jobs_lock`).
#
# A baseline taken beside place-and-route, synthesis or emulation is not a baseline: it fails load-sensitive
# tests that pass in isolation, and the result cannot be told apart from a real regression. Measured twice on
# 2026-09-11: the 05:31 baseline reported 70 failures at load 91 beside an `ours_flow` place-and-route, and the
# 20:31 run lost `bind_address_lock` at 181.8 s against its 180 s budget while `ours_flow` held ~830% CPU.
#
# A tool is matched by its EXECUTABLE NAME (`pgrep -x`), never by a command line (`-f`): a command line that
# merely mentions a tool (a watcher shell, a launcher, this check) is not that tool, and a `-f` match twice made
# a runner refuse against its own shell. Each pattern names a tool FAMILY: nextpnr installs one executable per
# architecture (nextpnr-xilinx, nextpnr-ice40, nextpnr-ecp5, nextpnr-himbaechel) and Quartus one per stage
# (quartus_sh, quartus_syn, quartus_fit, quartus_sta), so a bare `nextpnr` let xc7 routes run unseen beside a
# gate.
set -uo pipefail

# Each pattern is matched whole against the name the kernel records for the executable, which keeps at most 15
# characters: nextpnr-himbaechel is recorded as nextpnr-himbaec, still `nextpnr(-.*)?`.
HEAVY_EXECUTABLES=(
  'ours_flow'
  'yosys(-abc)?'
  'nextpnr(-.*)?'
  'vivado'
  'quartus_.*'
  'openFPGALoader'
  'xc7frames2bit'
  'qemu-riscv64'
  'qemu-aarch64'
  'openroad'
  'magic'
  'klayout'
  'netgen'
  'TritonRoute'
)

status=0

# The OpenLane RTL-to-GDSII flow and the Quartus builds run inside docker containers, found by their IMAGE NAME.
# `--filter ancestor=` compares against the reference as given, so `ancestor=ghcr.io/…/openlane` never matches a
# container started from `ghcr.io/…/openlane:<sha>-amd64`. A PAUSED container (`docker pause`, the cgroup
# freezer) runs nothing: it holds its memory and no CPU, so it is not a heavy job.
if command -v docker >/dev/null 2>&1; then
  while IFS= read -r container; do
    [ -n "$container" ] || continue
    echo "container $container"
    status=1
  done < <(sg docker -c "docker ps --format '{{.ID}} {{.Image}} {{.Status}}'" 2>/dev/null \
             | grep -E 'openlane|mpw_precheck|quartus' | grep -v '(Paused)')
fi

# A process inside a frozen cgroup (a paused container's tools) is not running: its cgroup's `cgroup.events`
# says `frozen 1`.
frozen() {
  local cg
  cg="$(sed -n 's/^0:://p' "/proc/$1/cgroup" 2>/dev/null)"
  [ -n "$cg" ] && grep -qx 'frozen 1' "/sys/fs/cgroup${cg}/cgroup.events" 2>/dev/null
}

for pattern in "${HEAVY_EXECUTABLES[@]}"; do
  while read -r pid name; do
    [ -n "${pid:-}" ] || continue
    frozen "$pid" && continue
    echo "$name (pid $pid): $(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null | cut -c1-160)"
    status=1
  done < <(pgrep -x -l "$pattern" 2>/dev/null)
done

exit "$status"
