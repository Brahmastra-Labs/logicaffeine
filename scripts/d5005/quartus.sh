#!/usr/bin/env bash
# The box's one Quartus queue. Every Quartus Prime Pro run goes through here: one at a time (a lock, and no Quartus
# container started outside the queue still running), in a container named for its job (`quartus.sh stop <job>`), with
# its processor count stated, its log at logs/quartus/<job>.log, and its reports recorded as a row of the build ledger
# (benchmarks/silicon/d5005/quartus_builds.json, by the quartus_record example) whether it passed or failed, judged
# on its exit status, its log and the flow modules it runs, never on its flow report alone.
#
#   quartus.sh syn    <job> <verilog> <top> [NAME=VALUE ...]              synthesis alone, every port a virtual pin
#   quartus.sh fit    <job> <verilog> <top> <period ns> [NAME=VALUE ...]  synthesis, fit and timing on one clock, clk
#   quartus.sh pr     <job> <work dir>                                    <work dir>/afu as a PR persona of the FIM
#   quartus.sh resume <job> <work dir>                                    a synthesized persona from its fitter on (from
#                                                                         its timing analysis once it is fitted); a
#                                                                         synthesized flat image (its FIM's work tree at
#                                                                         <work dir>/fim) held to the device, then from
#                                                                         its budget and fitter on
#   quartus.sh flat   <job> <work dir>                                    <work dir>/afu compiled into the FIM's own
#                                                                         source at <work dir>/ofs (build-afu.sh --flat),
#                                                                         the accelerator over the whole device: the
#                                                                         FIM's d5005 revision in <work dir>/fim
#   quartus.sh flat-syn <job> <work dir>                                  a flat build to its synthesis's end and its budget,
#                                                                         no fit: the report a floorplan is drawn from, its
#                                                                         fit then `resume`'s
#   quartus.sh sta    <job> <work dir> <tcl> [ARG ...]                    a fitted image's timing netlist analysed by a Tcl
#                                                                         script in the Timing Analyzer (quartus_sta -t),
#                                                                         given its project, its revision, the job's reports
#                                                                         directory <work dir>/sta/<job> and the ARGs: a flat
#                                                                         image's or a persona's
#   quartus.sh ls                                                         what waits and what runs
#   quartus.sh stop   <job>                                               stop a running job's container
#
# NAME=VALUE sets a top-level parameter. A job name is letters, digits, '-', '_' and '.', names one ledger row and one
# log; an analysis records no row (it builds nothing), its script kept beside its reports. Every Quartus tool checks a DDM token as it starts (DDM_T::verify_token in dni_dms's static initializer, "Cannot
# identify the client"), a check that fails for windows of minutes to hours and never once a tool has started: so each
# attempt waits until a probe of a tool's start-up passes, probing every
# QUARTUS_PROBE_WAIT seconds (default 60) for at most QUARTUS_STARTUP_LIMIT (default 21600) before it gives up on the
# job, and a module that dies of the check at its own start is run again, QUARTUS_ATTEMPTS times in all (default 8),
# from where the job stopped: a persona whose synthesis finished resumes at its fitter, and one whose fit succeeded
# since its synthesis at its timing analysis, the fit never run twice.
# QUARTUS_FLOORPLAN=<tcl> gives a fit a floorplan, named among its row's inputs: a fit job's is copied into the job as
# floorplan.tcl and sourced by its settings; a flat image's resume's is copied beside the FIM's project as
# logos_floorplan.tcl and sourced once by its d5005.qsf once the queue is taken, and a resume given none is placed by
# none, whatever an earlier resume was given. A floorplan is refused where it would place no fit: by a synthesis, a
# persona's build or resume (placed in the FIM's region), an image fitted since its synthesis, and a flat build (its
# work tree made in its container; its floorplan is drawn from a flat-syn and given to the resume).
# QUARTUS_PROCESSORS sets NUM_PARALLEL_PROCESSORS for syn and fit (default: every core) and, when set, for a PR build
# (whose template otherwise decides). QUARTUS_IMAGE (default logos-quartus-ofs:23.4-s10), Q234 (the 23.4 install,
# default /mnt/build/d5005-quartus/q234), QUARTUS_JOBS (job directories, default /mnt/build/d5005-quartus/jobs),
# QUARTUS_LOCK, QUARTUS_LEDGER (default the repository's ledger), QUARTUS_LOGS (default the repository's logs/quartus),
# QUARTUS_RECORD (a built quartus_record, else cargo runs it) and QUARTUS_BUDGET (a built d5005_budget, else cargo runs
# it) move the defaults. QUARTUS_DRY_RUN=1 writes a syn/fit job's project and prints the container command instead of
# taking the queue and running it.
# QUARTUS_LIB=1 with `source` defines the functions and runs nothing.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="${QUARTUS_IMAGE:-logos-quartus-ofs:23.4-s10}"
Q234="${Q234:-/mnt/build/d5005-quartus/q234}"
JOBS="${QUARTUS_JOBS:-/mnt/build/d5005-quartus/jobs}"
LOCK="${QUARTUS_LOCK:-/mnt/build/d5005-quartus/.queue.lock}"
LEDGER="${QUARTUS_LEDGER:-$ROOT/benchmarks/silicon/d5005/quartus_builds.json}"
LOGS="${QUARTUS_LOGS:-$ROOT/logs/quartus}"
ATTEMPTS="${QUARTUS_ATTEMPTS:-8}"
# Seconds a running attempt's container may go with neither its processes' CPU time nor the log moving before it is
# taken for hung (a tool deadlocked on its own pipes stays alive and silent), and how often both are looked at.
HANG_SECONDS="${QUARTUS_HANG_SECONDS:-1200}"
HANG_POLL="${QUARTUS_HANG_POLL:-60}"
PROBE_WAIT="${QUARTUS_PROBE_WAIT:-60}"
STARTUP_LIMIT="${QUARTUS_STARTUP_LIMIT:-21600}"
LICENSES="$ROOT/tools/d5005/licenses"
IP_CACHE="${QUARTUS_IP_CACHE:-/mnt/build/d5005-quartus/ip-cache}"
DEVICE=1SX280HN2F43E2VG
USAGE="usage: quartus.sh syn <job> <verilog> <top> [NAME=VALUE ...]
       quartus.sh fit <job> <verilog> <top> <period ns> [NAME=VALUE ...]
       quartus.sh pr <job> <work dir>
       quartus.sh resume <job> <work dir>
       quartus.sh flat <job> <work dir>
       quartus.sh flat-syn <job> <work dir>
       quartus.sh sta <job> <work dir> <tcl> [ARG ...]
       quartus.sh ls | stop <job>"

# A persona's flow: the FIM template's afu_synth, synthesis through the GBS.
PR_SCRIPT='set -euo pipefail; rm -rf /work/pr-template && cp -r /d5005/pr-template /work/pr-template; PROCESSORS
export OPAE_PLATFORM_ROOT=/work/pr-template
afu_synth_setup --sources /work/afu/hw/rtl/sources.txt --force /work/build
cd /work/build && $OPAE_PLATFORM_ROOT/bin/afu_synth'
PR_MODULES="Synthesis,Fitter,Timing Analyzer,Assembler"

# A synthesized persona from its fitter on, the flow's own module commands in its order, each run once across
# attempts (resume.done under the work directory holds the ones finished).
RESUME_SCRIPT='set -euo pipefail
export OPAE_PLATFORM_ROOT=/work/pr-template
cd /work/build/build/syn/syn_top
run() { grep -qx "$1" /work/resume.done 2>/dev/null || { "$@"; echo "$1" >> /work/resume.done; }; }
run quartus_fit --read_settings_files=on --write_settings_files=off d5005 -c iofs_pr_afu
run quartus_sta d5005 -c iofs_pr_afu --mode=finalize
run quartus_asm --read_settings_files=on --write_settings_files=off d5005 -c iofs_pr_afu
run quartus_sh -t ofs_partial_reconfig/gen_gbs.tcl dni_compile d5005 iofs_pr_afu
ls -la output_files/*.gbs'
RESUMED_MODULES="Fitter,Timing Analyzer,Assembler"

# A flat image's flow: the FIM's own build, its setup into /work/fim with the PIM and the accelerator's sources as the
# AFU the PIM wraps, then its IP, then its compile, the SOF its image (the FIM's finish only makes flash images, of no
# use to an image loaded over JTAG). The setup starts from no work tree — it asks before it builds into one an earlier
# attempt left, and gives up when no one answers — and from no module an earlier attempt marked run; once it is done it
# is marked so (resume.done). The IP is generated as its own step, the IP generation tool run until it finishes: a
# run's generation server dies or deadlocks at random, and each run generates only what the runs before left
# ungenerated (its regeneration policy skips the rest), so the compile's own IP generation finds it all generated; an
# IP that never generates stops the step after twenty runs. A setup keeps its tree's manifest and the hash of its IP
# sources (ip_cache.sh begin, in /work/ipcache), and the step restores the IP a build of the same sources and Quartus
# kept (QUARTUS_IP_CACHE, default /mnt/build/d5005-quartus/ip-cache) instead of generating it, every file checked, or
# generates it and, once the design built from it has synthesized, keeps what the generation wrote outside the project
# directory for the next build (scripts/d5005/ip_cache.sh). The compile then runs to its synthesis's end
# (ANALYSIS_AND_ELAB_ONLY, which the FIM's build_fim_compile.sh turns into `-end synthesis`), the design is held to the
# device's budget, and only then fitted, timed and assembled (FLAT_RESUME_SCRIPT's steps). From its IP on (FLAT_IP_SCRIPT)
# the work tree is as the setup left it.
#
# The design's synthesis estimates held to the device's, in the FIM's project directory: the ALMs it needs, its DSP
# blocks and its block memory bits, each summed over the report's partitions, against the 1SX280's 933,120 ALMs, 5,760
# DSP blocks and 11,721 M20Ks of 20,480 bits. A design the device cannot hold, or a report that gives no estimate, stops
# the attempt before its fitter (5), saying what it needs and what the device has; a fit of it would run for hours to fail.
# The bits undercount the blocks the fitter gives the memories (a stack a power of two deep, a byte-enabled block's 32
# bits of 40), so a flat image's resume and a flat synthesis are held on the host by the blocks too (`hold`, `budget`).
FLAT_BUDGET='r=output_files/d5005.syn.rpt
sum() { local t=0 n=0 v; for v in $(sed -n "s/^; $1 *; *\([0-9][0-9]*\) .*/\1/p" "$r" 2>/dev/null); do t=$((t + v)); n=$((n + 1)); done; if [ "$n" -gt 0 ]; then echo "$t"; fi; }
alms=$(sum "Estimate of Logic utilization (ALMs needed)")
dsps=$(sum "Total DSP Blocks")
bits=$(sum "Total block memory bits")
[ -n "$alms" ] && [ -n "$dsps" ] && [ -n "$bits" ] || { echo "quartus.sh: the synthesis report $r gives no estimate: no fit"; exit 5; }
if [ "$alms" -gt 933120 ] || [ "$dsps" -gt 5760 ] || [ "$bits" -gt 240046080 ]; then
  echo "quartus.sh: the design needs $alms ALMs, $dsps DSP blocks and $bits block memory bits; the device has 933120, 5760 and 240046080: no fit"
  exit 5
fi
echo "quartus.sh: the design needs $alms ALMs ($((100 * alms / 933120))%), $dsps DSP blocks ($((100 * dsps / 5760))%) and $bits block memory bits ($((100 * bits / 240046080))%)"'
# The rest of a flat fit whose retiming finished since its placement plan, and the plan since its synthesis: the
# fitter's finalize stage on the retimed snapshot (quartus_fit --finalize), the fit then marked run, so a fit stopped in
# its last stage goes on from it rather than placing and routing again. A finalize the fitter refuses (a retimed
# snapshot it cannot load) leaves the fit unmarked, to run whole; so is any other fit.
FLAT_FIT_REST='if ! grep -qx quartus_fit /work/resume.done 2>/dev/null \
  && [ output_files/d5005.fit.plan.rpt -nt output_files/d5005.syn.rpt ] \
  && [ output_files/d5005.fit.retime.rpt -nt output_files/d5005.fit.plan.rpt ]; then
  if quartus_fit --read_settings_files=on --write_settings_files=off d5005 -c d5005 --finalize > /work/finalize.log 2>&1; then
    cat /work/finalize.log
    echo quartus_fit >> /work/resume.done
  else
    echo "quartus.sh: the fitter refused to finalize the retimed snapshot ($(sed -n "s/^Error ([0-9]*): //p" /work/finalize.log | head -n 1)); the fit runs whole, the attempt in finalize.log"
  fi
fi'
# A synthesized flat image held to the device's budget, then the FIM flow's module commands in its order, each run once
# across attempts (resume.done under the work directory): the rest of a flat attempt from its synthesis on, and an
# attempt resumed after its synthesis.
FLAT_RESUME_SCRIPT='set -euo pipefail
cd /work/fim/syn/syn_top
run() { grep -qx "$1" /work/resume.done 2>/dev/null || { "$@"; echo "$1" >> /work/resume.done; }; }
'"$FLAT_BUDGET"'
'"$FLAT_FIT_REST"'
run quartus_fit --read_settings_files=on --write_settings_files=off d5005 -c d5005
run quartus_sta d5005 -c d5005 --mode=finalize
run quartus_asm --read_settings_files=on --write_settings_files=off d5005 -c d5005
ls -la /work/fim/syn/syn_top/output_files/d5005.sof'
# The compile ends at its synthesis by build_top.sh's -e, which sets the ANALYSIS_AND_ELAB_ONLY build_fim_compile.sh turns
# into `-end synthesis`: build_top.sh clears any ANALYSIS_AND_ELAB_ONLY it is handed, so the variable alone runs the
# whole flow, the fit among it.
FLAT_IP_SCRIPT='set -euo pipefail
cd /work/fim/syn/syn_top
# An IP generation hangs when a Tcl helper one of its generators starts dies of the DDM start-up check: the generator
# waits for the connection of the helper with no timeout, asleep, and the whole generation waits on it (twenty-two
# times on 10-27), its processes taking almost no CPU (about six seconds in three minutes with seven generators idle)
# where a working generation keeps cores busy (nine minutes and fifty seconds the longest one has taken). It is stopped
# once its processes take under thirty seconds of CPU and write nothing in three minutes, or at forty minutes in any
# case, and the loop runs it again.
quartus_ipgenerate() { /logos/stall_watch.sh 180 30 2400 "$(type -P quartus_ipgenerate)" "$@"; }
restored=1
if ! /logos/ip_cache.sh restore /work/fim /work/ipcache /ipcache; then
  restored=0
  n=0
  until quartus_ipgenerate --dni d5005 -c d5005 --run_default_mode_op; do
    n=$((n + 1)); [ "$n" -lt 20 ] || { echo "quartus.sh: the IP generation stopped 20 times; given up"; exit 3; }
    echo "quartus.sh: the IP generation stopped; again"
  done
fi
cd /work/ofs/ofs-d5005-ofs-2024.1-1
export OFS_ROOTDIR=$PWD OFS_PLATFORM_AFU_BBB=/work/ofs/ofs-platform-afu-bbb-ofs-2024.1-1 AFU_WITH_PIM=/work/afu/hw/rtl/sources.txt
# The synthesis asks the IP catalog for its search paths through the same Tcl helpers, and hangs the same way when one
# dies at its start; a working synthesis keeps cores busy or keeps logging (its pass over the IP it finds generated
# takes about sixteen CPU-seconds in five minutes), so it is stopped once its processes take under thirty seconds of
# CPU and write nothing in five minutes, or at four hours, and the attempt goes again.
/logos/stall_watch.sh 300 30 14400 ./ofs-common/scripts/common/syn/build_top.sh -e --stage=compile d5005 /work/fim
# The IP a generation wrote, kept once the design built from it has synthesized.
if [ "$restored" = 0 ] && ! /logos/ip_cache.sh store /work/fim /work/ipcache /ipcache; then echo "quartus.sh: the IP generated and not kept"; fi
'"$FLAT_RESUME_SCRIPT"
FLAT_SCRIPT="set -euo pipefail
cd /work/ofs/ofs-d5005-ofs-2024.1-1
export OFS_ROOTDIR=\$PWD OFS_PLATFORM_AFU_BBB=/work/ofs/ofs-platform-afu-bbb-ofs-2024.1-1 AFU_WITH_PIM=/work/afu/hw/rtl/sources.txt
rm -rf /work/fim /work/resume.done
./ofs-common/scripts/common/syn/build_top.sh --stage=setup d5005 /work/fim
PROCESSORS
/logos/ip_cache.sh begin /work/fim /work/ipcache
echo setup >> /work/resume.done
$FLAT_IP_SCRIPT"
FLAT_MODULES="Synthesis,Fitter,Timing Analyzer,Assembler"
# A flat synthesis: the flat build's setup, IP and synthesis, then the budget alone, never a fit; and its attempts from
# its IP generation, and from its synthesis done.
FLAT_SYN_BUDGET_SCRIPT='set -euo pipefail
cd /work/fim/syn/syn_top
'"$FLAT_BUDGET"
FLAT_SYN_IP_SCRIPT="${FLAT_IP_SCRIPT%"$FLAT_RESUME_SCRIPT"}$FLAT_SYN_BUDGET_SCRIPT"
FLAT_SYN_SCRIPT="${FLAT_SCRIPT%"$FLAT_IP_SCRIPT"}$FLAT_SYN_IP_SCRIPT"
FLAT_SYN_MODULES="Synthesis"


die() { echo "quartus.sh: $*" >&2; exit 2; }
container() { echo "logos-q-$1"; }
processors() { echo "${QUARTUS_PROCESSORS:-$(nproc)}"; }

check_job() {
  [[ "$1" =~ ^[A-Za-z0-9._-]+$ ]] || die "a job name is letters, digits, '-', '_' and '.': $1"
}

# Whether the job's row can be recorded once it has run: a built quartus_record, or cargo to run one. Asked before a
# real run takes the queue, so an hour of Quartus never ends with no row.
recordable() {
  if [ -n "${QUARTUS_RECORD:-}" ]; then
    [ -x "$QUARTUS_RECORD" ] || die "QUARTUS_RECORD is no executable: $QUARTUS_RECORD"
  else
    command -v cargo > /dev/null || die "no cargo on the PATH and no QUARTUS_RECORD: the job's row could not be recorded"
  fi
}

# Whether a flat image's synthesis can be held to the device on the host: a built d5005_budget, or cargo to run one.
# Asked before a real run takes the queue.
budgetable() {
  if [ -n "${QUARTUS_BUDGET:-}" ]; then
    [ -x "$QUARTUS_BUDGET" ] || die "QUARTUS_BUDGET is no executable: $QUARTUS_BUDGET"
  else
    command -v cargo > /dev/null || die "no cargo on the PATH and no QUARTUS_BUDGET: the synthesis could not be held to the device"
  fi
}

# The flat image whose synthesis report is `$1` held to the device by d5005_budget (`codegen_sva::preflight`): the ALMs
# and DSP blocks its partitions estimate and the M20K blocks the fitter gives its memories. Prints the verdict; its
# status is 5 for a design the device cannot hold or a report that gives no estimate.
budget() {
  if [ -n "${QUARTUS_BUDGET:-}" ]; then
    "$QUARTUS_BUDGET" "$1"
  else
    (cd "$ROOT" && CARGO_INCREMENTAL=0 cargo run -q -p logicaffeine-compile --example d5005_budget -- "$1")
  fi
}

# Hold the flat image whose synthesis report is `$1` to the device before its fitter (`budget`), the verdict printed and
# kept in `held` for the job's log; a design the device cannot hold ends the job (5) before it begins a log or takes the
# queue, its name free for the next, and a budget that could not run ends it too (2).
hold() {
  local s=0
  budgetable
  held=$(budget "$1") || s=$?
  [ -z "$held" ] || echo "$held"
  case "$s" in
    0) ;;
    5) echo "quartus.sh: $job is not fitted: the device cannot hold its synthesis ($1)" >&2; exit 5 ;;
    *) die "the synthesis $1 could not be held to the device: d5005_budget exited $s" ;;
  esac
}

# A job's log, which must not exist yet: a job name is used once.
new_log() {
  local log="$LOGS/$1.log"
  [ -e "$log" ] && die "job $1 has a log at $log; name a new job"
  echo "$log"
}

# The project of a syn or fit job: the design, its settings and, for a fit, its one clock.
write_project() {
  local mode=$1 job=$2 verilog=$3 top=$4 period=$5; shift 5
  local dir="$JOBS/$job"
  [ -e "$dir" ] && die "job $job exists at $dir; name a new job"
  [ -f "$verilog" ] || die "no Verilog at $verilog"
  mkdir -p "$dir"
  cp "$verilog" "$dir/design.v"
  {
    echo 'set_global_assignment -name FAMILY "Stratix 10"'
    echo "set_global_assignment -name DEVICE $DEVICE"
    echo "set_global_assignment -name TOP_LEVEL_ENTITY $top"
    echo 'set_global_assignment -name VERILOG_FILE design.v'
    echo 'set_global_assignment -name VERILOG_INPUT_VERSION SYSTEMVERILOG_2005'
    echo "set_global_assignment -name NUM_PARALLEL_PROCESSORS $(processors)"
    echo 'set_instance_assignment -name VIRTUAL_PIN ON -to *'
    if [ "$mode" = fit ]; then
      echo 'set_instance_assignment -name VIRTUAL_PIN OFF -to clk'
      echo 'set_global_assignment -name SDC_FILE design.sdc'
      [ -n "${QUARTUS_FLOORPLAN:-}" ] && echo 'set_global_assignment -name SOURCE_TCL_SCRIPT_FILE floorplan.tcl'
    fi
    for kv in "$@"; do
      [[ "$kv" =~ ^[A-Za-z_][A-Za-z0-9_]*=.+$ ]] || die "a parameter is NAME=VALUE: $kv"
      echo "set_parameter -name ${kv%%=*} ${kv#*=}"
    done
  } > "$dir/$job.qsf"
  echo "PROJECT_REVISION = \"$job\"" > "$dir/$job.qpf"
  if [ "$mode" = fit ]; then
    echo "create_clock -name clk -period $period [get_ports clk]" > "$dir/design.sdc"
    if [ -n "${QUARTUS_FLOORPLAN:-}" ]; then cp "$QUARTUS_FLOORPLAN" "$dir/floorplan.tcl"; fi
  fi
}

# The container command for a job: the install read-only, the job directory as /work, the licenses, the IP cache as
# /ipcache and this directory's scripts read-only as /logos.
run_command() {
  local job=$1 dir=$2 script=$3
  printf '%q ' docker run --rm --name "$(container "$job")" --network host -u "$(id -u):$(id -g)" -e HOME=/tmp \
    -v "$Q234:/opt/q234:ro" -v "$dir:/work" -v "$ROOT/tools/d5005:/d5005:ro" -v "$LICENSES:/lic:ro" \
    -v "$IP_CACHE:/ipcache" -v "$ROOT/scripts/d5005:/logos:ro" -e LM_LICENSE_FILE=/lic/LR-203385_License.dat:/lic/LR-203386_License.dat --entrypoint bash "$IMAGE" -c "$script"
  echo
}

# Take the queue: the lock, then no Quartus container from outside the queue.
take_queue() {
  local job=$1
  mkdir -p "$(dirname "$LOCK")"
  exec 9>"$LOCK"
  echo "quartus.sh: $job waits for the queue"
  flock 9
  while [ -n "$(docker ps -q --filter "ancestor=$IMAGE")" ]; do
    echo "quartus.sh: $job waits for $(docker ps --format '{{.Names}}' --filter "ancestor=$IMAGE" | tr '\n' ' ')"
    sleep 30
  done
  echo "$job $(date -u +%FT%TZ)" > "$LOCK.holder"
}

# Whether the attempt whose output starts at byte `mark` of `log` died of the DDM start-up check: the
# module's own message in that output, the stall watch's stop of a step standing still (a helper the check killed),
# or a crash report from the DMS manager's start-up written under `project` since the attempt began (`began`, seconds
# since the epoch). A step the watch stopped at its bound is a real failure.
transient() {
  local log=$1 mark=$2 project=$3 began=$4
  grep -qF 'Cannot identify the client' < <(tail -c +"$((mark + 1))" "$log") && return 0
  grep -q '^quartus.sh: .* hung: no CPU time and no log for' < <(tail -c +"$((mark + 1))" "$log") && return 0
  grep -q '^stall_watch.sh: .* s of CPU in .*; its processes stopped' < <(tail -c +"$((mark + 1))" "$log") && return 0
  [ -n "$(find "$project" -maxdepth 6 -name serv_req_info.txt -newermt "@$began" -exec grep -lF DMS_MANAGER {} + 2>/dev/null)" ]
}

# Whether the attempt whose output starts at byte `mark` of `log` finished synthesis.
synthesized() {
  grep -qF 'Quartus Prime Synthesis was successful' < <(tail -c +"$(($2 + 1))" "$1")
}

# The CPU seconds of every process in container `$1`, summed. Docker refuses ps columns that leave out the process ID.
container_cpu() {
  docker top "$1" -eo pid,time 2>/dev/null | tail -n +2 | awk '{ n = split($2, p, /[-:]/); s = 0; m = 1;
    for (i = n; i >= 1; i--) { s += p[i] * m; m = (i == n - 2 && n == 4) ? 86400 : m * 60 } t += s } END { print t + 0 }'
}

# One attempt of `script` in the job's container, its output appended to `log`; sets `status`, and `mark` and
# `began`, where in the log the attempt's output starts and when it started. An attempt whose container's CPU time and
# log both stand still for HANG_SECONDS is stopped and fails as hung (124), saying so in the log.
attempt() {
  local job=$1 dir=$2 log=$3
  mark=$(stat -c %s "$log")
  began=$(date +%s)
  mkdir -p "$IP_CACHE"
  set +e
  eval "$(run_command "$job" "$dir" "$script")" >> "$log" 2>&1 &
  local pid=$! cpu=-1 size=-1 still=0 hung=0 tick=0 c n
  while kill -0 "$pid" 2>/dev/null; do
    sleep 1
    tick=$((tick + 1))
    [ $((tick % HANG_POLL)) -eq 0 ] || continue
    kill -0 "$pid" 2>/dev/null || break
    c=$(container_cpu "$(container "$job")")
    n=$(stat -c %s "$log")
    if [ "$c" = "$cpu" ] && [ "$n" = "$size" ]; then still=$((still + HANG_POLL)); else still=0; fi
    cpu=$c
    size=$n
    if [ "$still" -ge "$HANG_SECONDS" ]; then
      echo "quartus.sh: $job hung: no CPU time and no log for $still s; its container stopped" >> "$log"
      hung=1
      docker stop "$(container "$job")" > /dev/null 2>&1
      break
    fi
  done
  wait "$pid"
  status=$?
  [ "$hung" = 1 ] && status=124
  set -e
}

# Whether a Quartus tool starts now in job `$1`'s container: its version printed, past the DDM start-up check, which
# ends a tool it fails with exit status 2.
startup_probe() {
  local job=$1
  docker run --rm --name "$(container "$job")-probe" --network host -u "$(id -u):$(id -g)" -e HOME=/tmp \
    -v "$Q234:/opt/q234:ro" -v "$LICENSES:/lic:ro" \
    -e LM_LICENSE_FILE=/lic/LR-203385_License.dat:/lic/LR-203386_License.dat --entrypoint bash "$IMAGE" \
    -c '/opt/q234/quartus/bin/quartus_sh --version' > /dev/null 2>&1
}

# Wait until a Quartus tool starts, probing every $PROBE_WAIT seconds for at most $STARTUP_LIMIT.
wait_for_startup() {
  local job=$1 log=$2 waited=0
  until startup_probe "$job"; do
    [ "$waited" -ge "$STARTUP_LIMIT" ] && die "no Quartus tool started in ${STARTUP_LIMIT}s: the DDM start-up check failed throughout; $job did not run"
    [ "$waited" = 0 ] && echo "quartus.sh: $job waits for the DDM start-up check to pass, probing every ${PROBE_WAIT}s" | tee -a "$log"
    sleep "$PROBE_WAIT"
    waited=$((waited + PROBE_WAIT))
  done
}

# Attempt `script`, each attempt once a Quartus tool starts, until it passes, fails for good, or has died of the DDM
# start-up check $ATTEMPTS times; between attempts the function `between` may change `script` and `expect`.
attempts() {
  local job=$1 dir=$2 log=$3 between=$4 n=1
  while :; do
    wait_for_startup "$job" "$log"
    attempt "$job" "$dir" "$log"
    [ "$status" = 0 ] && return 0
    transient "$log" "$mark" "$dir" "$began" || return 0
    [ "$n" -ge "$ATTEMPTS" ] && return 0
    echo "quartus.sh: $job died of the DDM start-up check or hung; attempt $((n + 1)) once a tool starts" | tee -a "$log"
    "$between"
    n=$((n + 1))
  done
}

# A synthesized persona's resume in `tree` (a flat image's, its reports `out` of revision `revision`): from its fitter,
# or, when its fit succeeded since its synthesis, from its timing analysis, the fit marked run (resume.done) and the
# modules expected of the job the rest.
resume_from() {
  local tree=$1 out="${2:-$1/build/build/syn/syn_top/output_files}" revision="${3:-iofs_pr_afu}"
  rm -f "$tree/resume.done"
  expect="$RESUMED_MODULES"
  if fitted "$out" "$revision"; then
    echo quartus_fit > "$tree/resume.done"
    expect="Timing Analyzer,Assembler"
  fi
}

# Whether the reports `$1` of revision `$2` say its fit succeeded since its synthesis.
fitted() {
  grep -q '^Fitter Status : Successful' "$1/$2.fit.summary" 2>/dev/null && [ "$1/$2.fit.summary" -nt "$1/$2.syn.summary" ]
}

# Between a persona's attempts: one whose synthesis finished goes on from its fitter, or from its timing analysis once
# it is fitted. A job whose own earlier attempt ran the fitter is judged without it and names itself resumed.
pr_between() {
  if [ "$script" != "$RESUME_SCRIPT" ] && synthesized "$log" "$mark"; then
    script="$RESUME_SCRIPT"
    resume_from "$tree"
    case ",$expect," in *,Fitter,*) ;; *) resumed="${job:-}" ;; esac
  elif [ "$script" = "$RESUME_SCRIPT" ] && [[ ",$expect," == *,Fitter,* ]] && grep -qx quartus_fit "$tree/resume.done" 2>/dev/null; then
    expect="Timing Analyzer,Assembler"
    resumed="${job:-}"
  fi
}

# Between a flat image's attempts: one whose synthesis finished goes on from its fitter, or from its timing analysis
# once it is fitted.
flat_between() {
  if [ "$script" != "$FLAT_RESUME_SCRIPT" ] && synthesized "$log" "$mark"; then
    script="$FLAT_RESUME_SCRIPT"
    resume_from "$tree" "$tree/fim/syn/syn_top/output_files" d5005
  elif [ "$script" != "$FLAT_RESUME_SCRIPT" ] && grep -qx setup "$tree/resume.done" 2>/dev/null; then
    script="$FLAT_IP_SCRIPT"
  fi
}

# Where a flat synthesis's next attempt goes on from: the budget alone once its synthesis is done, its IP generation
# once its setup is, else from no work tree; never a fit.
flat_syn_between() {
  if synthesized "$log" "$mark"; then
    script="$FLAT_SYN_BUDGET_SCRIPT"
  elif grep -qx setup "$tree/resume.done" 2>/dev/null; then
    script="$FLAT_SYN_IP_SCRIPT"
  fi
}

# The FIM source tree of flat build tree `$1`, its PIM beside it and its floorplan whole-device; dies naming what is
# missing.
flat_fim() {
  local tree=$1 fim="$1/ofs/ofs-d5005-ofs-2024.1-1"
  [ -f "$tree/afu/hw/rtl/sources.txt" ] || die "no AFU tree at $tree/afu (build-afu.sh writes it)"
  [ -f "$fim/ofs-common/scripts/common/syn/build_top.sh" ] && [ -f "$fim/syn/setup/sr_logic_lock_region.tcl" ] \
    && [ -d "$tree/ofs/ofs-platform-afu-bbb-ofs-2024.1-1/plat_if_develop/ofs_plat_if" ] \
    || die "no FIM source tree with its PIM at $tree/ofs (build-afu.sh --flat writes it)"
  if grep -qE '^[[:space:]]*set_(global|instance)_assignment' "$fim/syn/setup/sr_logic_lock_region.tcl"; then
    die "the FIM's floorplan $fim/syn/setup/sr_logic_lock_region.tcl still reserves the persona region for afu_main (scripts/d5005/flat-fim.sh makes it whole-device)"
  fi
  echo "$fim"
}

# The floorplan flat build tree `$1`'s fit is placed by: QUARTUS_FLOORPLAN copied beside the FIM's project as
# logos_floorplan.tcl and sourced once by its d5005.qsf, after the project's own settings; given none, none, whatever an
# earlier resume of the tree was given. The project is rewritten only when it sources a floorplan to drop.
place_flat() {
  local top="$1/fim/syn/syn_top" line='set_global_assignment -name SOURCE_TCL_SCRIPT_FILE logos_floorplan.tcl'
  if [ -f "$top/d5005.qsf" ] && grep -qxF "$line" "$top/d5005.qsf"; then
    awk -v l="$line" '$0 != l' "$top/d5005.qsf" > "$top/d5005.qsf.placing"
    mv "$top/d5005.qsf.placing" "$top/d5005.qsf"
  fi
  rm -f "$top/logos_floorplan.tcl"
  if [ -n "${QUARTUS_FLOORPLAN:-}" ]; then
    cp "$QUARTUS_FLOORPLAN" "$top/logos_floorplan.tcl"
    echo "$line" >> "$top/d5005.qsf"
  fi
}

# A synthesized flat image's resume, the queue taken and the log begun: its budget and fit on from its synthesis (or its
# timing analysis once it is fitted), attempted and recorded as a flat build is, and remembered as a persona's resume is.
resume_flat() {
  local job=$1 tree=$2
  place_flat "$tree"
  resume_from "$tree" "$tree/fim/syn/syn_top/output_files" d5005
  attempts "$job" "$tree" "$log" flat_between
  tail -n 3 "$log"
  mapfile -t inputs < <({
    find "$tree/afu/hw/rtl" -maxdepth 1 -type f
    echo "$tree/ofs/ofs-d5005-ofs-2024.1-1/syn/setup/sr_logic_lock_region.tcl"
    if [ -n "${QUARTUS_FLOORPLAN:-}" ]; then echo "$tree/fim/syn/syn_top/logos_floorplan.tcl"; fi
  } | sort)
  record "$tree/fim/syn/syn_top/output_files" d5005 "$job" "$log" "${inputs[@]}"
  case ",$expect," in *,Fitter,*) remember "$tree" "$job" ;; esac
  if [ "${resumed:-}" = "$job" ]; then remember "$tree" "$job"; fi
}

# Leave in a persona's tree the job whose compile ran its fitter, the compile a later resume goes on from.
remember() {
  local tree=$1 job=$2
  echo "$job" > "$tree/compiled.by"
}

# The compile a resume of the tree goes on from, as the last compile there that ran the fitter left it; nothing if none
# did.
resumed_from() {
  local tree=$1
  if [ -s "$tree/compiled.by" ]; then cat "$tree/compiled.by"; fi
}

# Record the job's reports, judged on the last attempt: its exit status, its own output, the flow modules the job
# runs, and the time the job started. The recorder and the budget build without incremental state, whose sessions in
# the shared target a concurrent build may move.
record() {
  local reports=$1 revision=$2 job=$3 log=$4; shift 4
  local last
  last=$(mktemp)
  tail -c +"$((mark + 1))" "$log" > "$last"
  local args=(--exit "$status" --log "$last" --expect "$expect" --started "$since")
  [ -n "${resumed:-}" ] && args+=(--resumed "$resumed")
  args+=("$reports" "$revision" "$job" "$LEDGER" "$@")
  local recorded=0
  if [ -n "${QUARTUS_RECORD:-}" ]; then
    "$QUARTUS_RECORD" "${args[@]}" || recorded=$?
  else
    (cd "$ROOT" && CARGO_INCREMENTAL=0 cargo run -q -p logicaffeine-compile --example quartus_record -- "${args[@]}") || recorded=$?
  fi
  rm -f "$last"
  [ "$recorded" = 0 ] || die "the ledger did not record $job (quartus_record exited $recorded); its log is $log"
}

[ "${QUARTUS_LIB:-}" = 1 ] && return 0

# The dispatch is one function, read whole before it runs, and its call and the exit after it one line, so an edit
# to this file while a job runs never reaches the running job.
main() {
  local cmd="${1:-}"; [ $# -eq 0 ] || shift
  case "$cmd" in
    syn|fit)
      if [ "$cmd" = fit ]; then
        [ $# -ge 4 ] || die "$USAGE"
        job=$1 verilog=$2 top=$3 period=$4; shift 4
        [[ "$period" =~ ^[0-9]+(\.[0-9]+)?$ ]] || die "a period is nanoseconds: $period"
        modules=(quartus_syn quartus_fit quartus_sta)
        expect="Synthesis,Fitter,Timing Analyzer"
      else
        [ $# -ge 3 ] || die "$USAGE"
        job=$1 verilog=$2 top=$3 period=-; shift 3
        modules=(quartus_syn)
        expect="Synthesis"
      fi
      check_job "$job"
      if [ -n "${QUARTUS_FLOORPLAN:-}" ]; then
        [ "$cmd" = fit ] || die "a synthesis places nothing: QUARTUS_FLOORPLAN names a floorplan for a fit"
        [ -f "$QUARTUS_FLOORPLAN" ] || die "no floorplan at $QUARTUS_FLOORPLAN"
      fi
      [ "${QUARTUS_DRY_RUN:-}" = 1 ] || { recordable; log=$(new_log "$job"); }
      verilog="$(cd "$(dirname "$verilog")" && pwd)/$(basename "$verilog")"
      write_project "$cmd" "$job" "$verilog" "$top" "$period" "$@"
      # Each module runs once across attempts; modules.done in the job directory holds the ones finished.
      script="cd /work && touch modules.done && run() { grep -qx \"\$1\" modules.done || { \"\$@\" && echo \"\$1\" >> modules.done; }; }"
      for m in "${modules[@]}"; do script+=" && run $m $job"; done
      if [ "${QUARTUS_DRY_RUN:-}" = 1 ]; then
        run_command "$job" "$JOBS/$job" "$script"
        exit 0
      fi
      mkdir -p "$LOGS" && : > "$log"
      take_queue "$job"
      since=$(date +%s)
      attempts "$job" "$JOBS/$job" "$log" :
      tail -n 1 "$log"
      inputs=("$JOBS/$job/design.v" "$JOBS/$job/$job.qsf")
      [ -f "$JOBS/$job/design.sdc" ] && inputs+=("$JOBS/$job/design.sdc")
      [ -f "$JOBS/$job/floorplan.tcl" ] && inputs+=("$JOBS/$job/floorplan.tcl")
      record "$JOBS/$job" "$job" "$job" "$log" "${inputs[@]}"
      ;;
    pr|resume)
      [ $# -eq 2 ] || die "$USAGE"
      job=$1; tree="$(cd "$2" && pwd)"
      check_job "$job"
      [ -f "$tree/afu/hw/rtl/sources.txt" ] || die "no AFU tree at $tree/afu (build-afu.sh writes it)"
      procs=""
      [ -n "${QUARTUS_PROCESSORS:-}" ] && procs="echo 'set_global_assignment -name NUM_PARALLEL_PROCESSORS $QUARTUS_PROCESSORS' >> /work/pr-template/hw/lib/build/syn/syn_top/iofs_pr_afu.qsf"
      flat=0
      if [ "$cmd" = pr ]; then
        script="${PR_SCRIPT/PROCESSORS/$procs}"
        expect="$PR_MODULES"
      elif [ -d "$tree/ofs/ofs-d5005-ofs-2024.1-1" ]; then
        # A flat tree (build-afu.sh --flat): its FIM's work tree as its synthesis left it, from the budget on.
        [ -d "$tree/fim/syn/syn_top/qdb" ] || die "no synthesized flat image under $tree/fim (quartus.sh flat synthesizes one)"
        flat=1
        resumed=$(resumed_from "$tree")
        script="$FLAT_RESUME_SCRIPT"
        expect="$RESUMED_MODULES"
      else
        [ -d "$tree/build/build/syn/syn_top/qdb" ] || die "no synthesized persona under $tree/build (quartus.sh pr synthesizes one)"
        resumed=$(resumed_from "$tree")
        script="$RESUME_SCRIPT"
        expect="$RESUMED_MODULES"
      fi
      if [ -n "${QUARTUS_FLOORPLAN:-}" ]; then
        [ "$flat" = 1 ] || die "a persona is placed in the FIM's region: QUARTUS_FLOORPLAN places a flat image's fit"
        [ -f "$QUARTUS_FLOORPLAN" ] || die "no floorplan at $QUARTUS_FLOORPLAN"
        [ -f "$tree/fim/syn/syn_top/d5005.qsf" ] || die "no FIM project at $tree/fim/syn/syn_top/d5005.qsf for the floorplan to place"
        if fitted "$tree/fim/syn/syn_top/output_files" d5005; then die "$tree is fitted since its synthesis: a floorplan places no fit that is done"; fi
      fi
      if [ "${QUARTUS_DRY_RUN:-}" = 1 ]; then
        run_command "$job" "$tree" "$script"
        exit 0
      fi
      recordable
      log=$(new_log "$job")
      if [ "$flat" = 1 ] && ! fitted "$tree/fim/syn/syn_top/output_files" d5005; then hold "$tree/fim/syn/syn_top/output_files/d5005.syn.rpt"; fi
      mkdir -p "$LOGS" && : > "$log"
      [ -z "${held:-}" ] || echo "$held" >> "$log"
      take_queue "$job"
      since=$(date +%s)
      [ "$flat" = 1 ] && { resume_flat "$job" "$tree"; exit 0; }
      [ "$cmd" = resume ] && resume_from "$tree"
      attempts "$job" "$tree" "$log" pr_between
      tail -n 3 "$log"
      mapfile -t inputs < <(find "$tree/afu/hw/rtl" -maxdepth 1 -type f | sort)
      record "$tree/build/build/syn/syn_top/output_files" iofs_pr_afu "$job" "$log" "${inputs[@]}"
      case ",$expect," in *,Fitter,*) remember "$tree" "$job" ;; esac
      [ "${resumed:-}" = "$job" ] && remember "$tree" "$job" || :
      ;;
    flat)
      [ $# -eq 2 ] || die "$USAGE"
      job=$1; tree="$(cd "$2" && pwd)"
      check_job "$job"
      [ -z "${QUARTUS_FLOORPLAN:-}" ] || die "a flat build makes its work tree in its container: its floorplan is drawn from a flat-syn and given to its resume"
      fim=$(flat_fim "$tree")
      procs=""
      [ -n "${QUARTUS_PROCESSORS:-}" ] && procs="echo 'set_global_assignment -name NUM_PARALLEL_PROCESSORS $QUARTUS_PROCESSORS' >> /work/fim/syn/syn_top/d5005.qsf"
      script="${FLAT_SCRIPT/PROCESSORS/$procs}"
      expect="$FLAT_MODULES"
      if [ "${QUARTUS_DRY_RUN:-}" = 1 ]; then
        run_command "$job" "$tree" "$script"
        exit 0
      fi
      recordable; log=$(new_log "$job")
      mkdir -p "$LOGS" && : > "$log"
      take_queue "$job"
      since=$(date +%s)
      attempts "$job" "$tree" "$log" flat_between
      tail -n 3 "$log"
      mapfile -t inputs < <({ find "$tree/afu/hw/rtl" -maxdepth 1 -type f; echo "$fim/syn/setup/sr_logic_lock_region.tcl"; } | sort)
      record "$tree/fim/syn/syn_top/output_files" d5005 "$job" "$log" "${inputs[@]}"
      case ",$expect," in *,Fitter,*) remember "$tree" "$job" ;; esac
      ;;
    flat-syn)
      [ $# -eq 2 ] || die "$USAGE"
      job=$1; tree="$(cd "$2" && pwd)"
      check_job "$job"
      [ -z "${QUARTUS_FLOORPLAN:-}" ] || die "a synthesis places nothing: QUARTUS_FLOORPLAN places the resume of a flat-syn"
      fim=$(flat_fim "$tree")
      procs=""
      [ -n "${QUARTUS_PROCESSORS:-}" ] && procs="echo 'set_global_assignment -name NUM_PARALLEL_PROCESSORS $QUARTUS_PROCESSORS' >> /work/fim/syn/syn_top/d5005.qsf"
      script="${FLAT_SYN_SCRIPT/PROCESSORS/$procs}"
      expect="$FLAT_SYN_MODULES"
      if [ "${QUARTUS_DRY_RUN:-}" = 1 ]; then
        run_command "$job" "$tree" "$script"
        exit 0
      fi
      recordable; budgetable; log=$(new_log "$job")
      mkdir -p "$LOGS" && : > "$log"
      take_queue "$job"
      since=$(date +%s)
      attempts "$job" "$tree" "$log" flat_syn_between
      if [ "$status" = 0 ]; then budget "$tree/fim/syn/syn_top/output_files/d5005.syn.rpt" >> "$log" || status=$?; fi
      tail -n 3 "$log"
      mapfile -t inputs < <({ find "$tree/afu/hw/rtl" -maxdepth 1 -type f; echo "$fim/syn/setup/sr_logic_lock_region.tcl"; } | sort)
      record "$tree/fim/syn/syn_top/output_files" d5005 "$job" "$log" "${inputs[@]}"
      ;;
    sta)
      [ $# -ge 3 ] || die "$USAGE"
      job=$1; tree="$(cd "$2" && pwd)"; tcl=$3; shift 3
      check_job "$job"
      [ -z "${QUARTUS_FLOORPLAN:-}" ] || die "an analysis places nothing: QUARTUS_FLOORPLAN places a fit"
      [ -f "$tcl" ] || die "no Tcl script at $tcl"
      if [ -d "$tree/ofs/ofs-d5005-ofs-2024.1-1" ]; then top=fim/syn/syn_top revision=d5005; else top=build/build/syn/syn_top revision=iofs_pr_afu; fi
      fitted "$tree/$top/output_files" "$revision" || die "no fitted image under $tree/$top: an analysis reads the timing netlist of a fit"
      out="sta/$job"
      [ -e "$tree/$out" ] && die "job $job has reports at $tree/$out; name a new job"
      script="set -euo pipefail
cd /work/$top
quartus_sta -t /work/$out/analysis.tcl d5005 $revision /work/$out"
      for a in "$@"; do script+=" $(printf %q "$a")"; done
      if [ "${QUARTUS_DRY_RUN:-}" = 1 ]; then
        run_command "$job" "$tree" "$script"
        exit 0
      fi
      log=$(new_log "$job")
      mkdir -p "$tree/$out" && cp "$tcl" "$tree/$out/analysis.tcl"
      mkdir -p "$LOGS" && : > "$log"
      take_queue "$job"
      attempts "$job" "$tree" "$log" :
      tail -n 3 "$log"
      exit "$status"
      ;;
    ls)
      [ -f "$LOCK.holder" ] && echo "last to take the queue: $(cat "$LOCK.holder")"
      docker ps --format '{{.Names}}\t{{.Status}}' --filter "ancestor=$IMAGE"
      ;;
    stop)
      [ $# -eq 1 ] || die "$USAGE"
      check_job "$1"
      docker stop "$(container "$1")"
      ;;
    *)
      die "$USAGE"
      ;;
  esac
}

main "$@"; exit
