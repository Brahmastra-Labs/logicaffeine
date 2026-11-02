#!/usr/bin/env bash
# One test of a full run, recorded. The full profile (.config/nextest.toml) runs every test through this wrapper,
# which runs the test under GNU time and appends one line to the run's ledger, $LOGOS_TEST_LEDGER: the run, the
# binary, the test, when it started and ended, its wall clock, its CPU (user and system, with every process it waited
# for), its peak resident set, and how it ended (`exit N` or `signal N`). scripts/test/test-ledger-report.py reads a
# run's load from the ledger. Without a ledger, or off Linux, it only runs the test.
#
# The test is the wrapper's to the end: its arguments, its output, its exit status and the signal that ended it come
# back as they left it, so nextest reports a failure, a crash or a timeout as it would unwrapped. nextest ends a test
# by signalling its process group, which holds this wrapper too: the wrapper catches the signal, lets the test end,
# records it, and ends by the same signal.
ledger="${LOGOS_TEST_LEDGER:-}"
unset LOGOS_TEST_LEDGER
if [[ -z "$ledger" || "$OSTYPE" != linux* || ! -x /usr/bin/time ]]; then
  exec "$@"
fi

trap : TERM INT HUP QUIT
rusage="$(mktemp "${TMPDIR:-/tmp}/test-ledger.XXXXXX")" || exec "$@"
start=${EPOCHREALTIME/,/.}
/usr/bin/time -o "$rusage" -f '%U %S %M' -- "$@"
status=$?
end=${EPOCHREALTIME/,/.}

signal="" last=""
while IFS= read -r line; do
  case "$line" in
    "Command terminated by signal "*) signal="${line##* }" ;;
    "Command exited with non-zero status "*) ;;
    *) last="$line" ;;
  esac
done < "$rusage"
rm -f "$rusage"

span=$(( 10#${end/./} - 10#${start/./} ))
printf -v wall '%d.%06d' $(( span / 1000000 )) $(( span % 1000000 ))
read -r user sys rss <<< "$last"
if [[ -z "$rss" ]]; then
  # GNU time itself was ended (the group's signal reached it before it reaped the test): the signal that ended it
  # ended the test.
  user=- sys=- rss=-
  (( status > 128 )) && signal=$(( status - 128 ))
fi
if [[ -n "$signal" ]]; then outcome="signal $signal"; else outcome="exit $status"; fi

printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
  "${NEXTEST_RUN_ID:--}" "${NEXTEST_BINARY_ID:--}" "${NEXTEST_TEST_NAME:--}" \
  "$start" "$end" "$wall" "$user" "$sys" "$rss" "$outcome" >> "$ledger"

if [[ -n "$signal" ]]; then
  trap - "$signal"
  builtin kill -"$signal" "$$"
fi
exit "$status"
