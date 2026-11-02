#!/usr/bin/env bash
# llama.cpp's BATCHED generation throughput, via llama-server's parallel slots.
#
# `llama-batched-bench` segfaults on this checkpoint (immediately, no output, even at -npl 1), while
# llama-cli, llama-bench and a direct llama_decode all drive the model fine — so this measures the same
# thing through the path llama.cpp is actually deployed with. N requests run concurrently against N slots;
# each request's reported `predicted_per_second` already reflects the concurrency, so their SUM is the
# aggregate generation throughput -- the quantity our AGGREGATE_TOK_S reports.
set -uo pipefail
W=/workspace
M=$W/qwen38/Qwen3.8-27B-Q8_0.gguf
BIN=$W/llama.cpp/build/bin
PORT=${PORT:-8919}
NGEN=${NGEN:-128}
BATCHES=${BATCHES:-1 2 4 8 16 32}
say(){ echo "[$(date -u +%H:%M:%S)] $*"; }

for N in $BATCHES; do
  say "--- llama.cpp server, $N parallel slots ---"
  $BIN/llama-server -m "$M" -ngl 99 -np "$N" -c $(( N * 1024 )) --port $PORT --host 127.0.0.1 \
      > $W/server_$N.log 2>&1 &
  SPID=$!
  for i in $(seq 120); do
    curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break
    sleep 2
    kill -0 $SPID 2>/dev/null || { say "server died; last lines:"; tail -6 $W/server_$N.log; break; }
  done
  if ! curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    say "!! server not healthy at np=$N"; kill $SPID 2>/dev/null; wait $SPID 2>/dev/null; continue
  fi
  rm -f $W/resp_*.json
  for i in $(seq "$N"); do
    curl -s "http://127.0.0.1:$PORT/completion" -H 'Content-Type: application/json' \
      -d "{\"prompt\":\"The capital of France is\",\"n_predict\":$NGEN,\"temperature\":0,\"cache_prompt\":false}" \
      > $W/resp_$i.json 2>/dev/null &
  done
  wait
  python3 - "$N" <<'PY'
import glob, json, sys
n = int(sys.argv[1]); tot = 0.0; toks = 0; ok = 0
for f in glob.glob('/workspace/resp_*.json'):
    try:
        d = json.load(open(f)); t = d.get('timings', {})
        if 'predicted_per_second' in t:
            tot += t['predicted_per_second']; toks += t.get('predicted_n', 0); ok += 1
    except Exception: pass
print("LLAMA_BATCH %d  responses %d  AGGREGATE_TOK_S %.2f  (tokens %d)" % (n, ok, tot, toks))
PY
  kill $SPID 2>/dev/null; wait $SPID 2>/dev/null
  sleep 3
done
say "=== LLAMA BATCHED BASELINE DONE ==="
