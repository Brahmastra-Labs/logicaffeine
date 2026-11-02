#!/usr/bin/env bash
# Answer "is this actually disk?" and reclaim build caches that nothing is using.
#
# Three sessions misread the same ENOSPC as disk pressure in one night. It was not always disk: `unable to
# copy .../target/debug/incremental/...  No space left on device` appeared with 1.3T free and inodes at 2%,
# and `CARGO_INCREMENTAL=0` made it go away. So the first job here is to say which of the three possible
# causes is real — space, inodes, or neither — rather than leave it to be guessed from the message.
#
#   scripts/dev/disk-guard.sh                 report only
#   scripts/dev/disk-guard.sh --sweep [DAYS]  also remove stale build caches (default 7 days), never live ones
set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1

DAYS=7
SWEEP=0
[ "${1-}" = "--sweep" ] && { SWEEP=1; [ -n "${2-}" ] && DAYS="$2"; }

fs=$(df -h . | awk 'NR==2')
inode=$(df -i . | awk 'NR==2')
space_pct=$(echo "$fs" | awk '{gsub(/%/,"",$5); print $5}')
inode_pct=$(echo "$inode" | awk '{gsub(/%/,"",$5); print $5}')

echo "── filesystem ──"
echo "  space   $(echo "$fs" | awk '{print $4}') free of $(echo "$fs" | awk '{print $2}')  (${space_pct}% used)"
echo "  inodes  $(echo "$inode" | awk '{print $4}') free  (${inode_pct}% used)"

# A real write, because both counters can look fine while the allocator still refuses. This is the check that
# separates "the disk is full" from "something else returned ENOSPC".
probe=$(mktemp -p . .disk-guard.XXXXXX 2>/dev/null) || probe=""
verdict="unknown"
if [ -n "$probe" ]; then
    if dd if=/dev/zero of="$probe" bs=1M count=64 status=none 2>/dev/null; then
        verdict="writes fine"
    else
        verdict="WRITES FAILING"
    fi
    rm -f "$probe"
fi
echo "  64 MiB write probe: $verdict"

echo
if [ "$space_pct" -ge 90 ]; then
    echo "  VERDICT: space. ${space_pct}% used — reclaim below."
elif [ "$inode_pct" -ge 90 ]; then
    echo "  VERDICT: inodes. ${inode_pct}% used with space to spare — many small files, not big ones."
elif [ "$verdict" = "WRITES FAILING" ]; then
    echo "  VERDICT: neither space nor inodes, yet writes fail. Suspect a quota, a cgroup limit, or"
    echo "           free-space fragmentation. Try CARGO_INCREMENTAL=0 first — it avoided this all of"
    echo "           the times we hit it, because incremental copies whole object files around."
else
    echo "  VERDICT: not disk. If a build says ENOSPC, it is not this filesystem — check the message"
    echo "           against a build with CARGO_INCREMENTAL=0 before blaming space."
fi

echo
echo "── build caches, largest first ──"
for d in target/_*_iso* target/*-toolchain target/*_target; do
    [ -d "$d" ] || continue
    age=$(( ( $(date +%s) - $(stat -c %Y "$d") ) / 86400 ))
    open=$(lsof +D "$d" 2>/dev/null | tail -n +2 | wc -l)
    printf "  %-36s %6s  %3dd old  %s\n" "$d" "$(du -sh "$d" 2>/dev/null | cut -f1)" "$age" \
        "$( [ "$open" -gt 0 ] && echo "IN USE ($open handles)" || echo "idle" )"
done
echo "  (target/debug and target/release are never swept here: a live build is almost always in them)"

if [ "$SWEEP" = 1 ]; then
    echo
    echo "── sweeping caches idle for more than ${DAYS}d ──"
    freed=0
    for d in $(find target -maxdepth 1 -type d \( -name "_*_iso*" -o -name "*_target" \) -mtime "+$DAYS" 2>/dev/null); do
        if [ "$(lsof +D "$d" 2>/dev/null | tail -n +2 | wc -l)" -gt 0 ]; then
            echo "  skipped $d — a process still holds it open"
            continue
        fi
        sz=$(du -sh "$d" 2>/dev/null | cut -f1)
        rm -rf "$d" && { echo "  removed $d ($sz)"; freed=$((freed + 1)); }
    done
    [ "$freed" = 0 ] && echo "  nothing idle enough to sweep"
    echo "  space now: $(df -h . | awk 'NR==2{print $4}') free"
fi
