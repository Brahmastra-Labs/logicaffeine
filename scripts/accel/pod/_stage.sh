#!/usr/bin/env bash
# Shared pod-session staging: the volume-throughput gate, local-NVMe staging, and warming.
#
# Extracted verbatim from session-qwen38.sh so the vLLM head-to-head does not re-derive it. The gate is
# not optional hygiene — it is the fix for a session that rented an H100 and spent it watching a network
# volume deliver 1.8 MB/s (it had served ~50 MB/s earlier the SAME day). A 29 GB load at that rate is
# four hours. Measure BEFORE committing rented GPU time.
#
# Source it, then call:
#   say <msg>                      timestamped log line
#   vol_gate <probe-file>          exits 3 if the volume reads slower than $VOL_MIN_MBS (default 20)
#   stage_local <src> <dst> <gb>   copies src->dst when the local disk has more than <gb> free
#   warm_files <dir> [file...]     parallel cat so the first real read is not a cold read

VOL_MIN_MBS=${VOL_MIN_MBS:-20}

say() { echo "[$(date -u +%H:%M:%S)] $*"; }

vol_gate() {
  local probe="$1" mbs
  if [ ! -f "$probe" ]; then say "!! volume gate: probe file $probe is missing"; return 1; fi
  mbs=$(dd if="$probe" of=/dev/null bs=1M count=400 2>&1 | tail -1 | grep -oE "[0-9.]+ [MG]B/s" | head -1)
  say "volume read speed: ${mbs:-unknown}"
  case "$mbs" in
    *GB/s) return 0 ;;
    *MB/s) local v=${mbs%% *}
           if [ "${v%%.*}" -lt "$VOL_MIN_MBS" ] 2>/dev/null; then
             say "!! volume is degraded (${mbs}); a 29 GB load would take hours. Not starting."
             say "!! retry later, or stage weights while the volume is healthy."
             exit 3
           fi ;;
    *) say "!! could not measure the volume; proceeding cautiously" ;;
  esac
}

stage_local() {
  local src="$1" dst="$2" need_gb="$3" free
  free=$(df -BG --output=avail / | tail -1 | tr -dc '0-9')
  if [ "${free:-0}" -gt "$need_gb" ] && [ ! -e "$dst" ]; then
    say "staging $src -> $dst (${free} GB free locally)"
    mkdir -p "$dst" && cp -r "$src/." "$dst/" && say "staged to $dst"
  else
    say "not staging $src (free=${free:-?} GB, need >${need_gb}, dst exists: $([ -e "$dst" ] && echo yes || echo no))"
  fi
}

warm_files() {
  local dir="$1"; shift
  say "warming $dir"
  find "$dir" -type f -print0 2>/dev/null | xargs -0 -P 24 -I{} cat {} > /dev/null 2>&1
  for extra in "$@"; do [ -f "$extra" ] && cat "$extra" > /dev/null 2>&1; done
  say "warm"
}
