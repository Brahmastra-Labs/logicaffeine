#!/usr/bin/env bash
# SAMBACLOUD SMOKE — one streamed chat completion from SambaNova Cloud, timed: prints ONE JSON line with the
# time to first token (TTFT, from the request going out to the first `data:` chunk) and the tokens per second
# over the streamed span. The baseline number the SN50 day starts from.
#
#   scripts/accel/sambacloud-smoke.sh              stream one completion, print the JSON line
#   scripts/accel/sambacloud-smoke.sh --record     also write benchmarks/results/sambacloud_baseline.json (dated)
#   scripts/accel/sambacloud-smoke.sh --dry-run    print the request shape (key REDACTED); no network
#   scripts/accel/sambacloud-smoke.sh --help
#
# The key is read from SAMBANOVA_API_KEY, else ~/.sambanova_key. It is NEVER printed and never placed on a
# command line (curl reads it from a config file descriptor). SAMBANOVA_MODEL selects the model
# (default MiniMax-M2.7); SAMBANOVA_URL the endpoint.
#
# SAMBANOVA_STREAM_FILE replays a captured SSE stream through the same measuring path (no key, no network);
# its record is stamped `replayed_sse_capture` so it can never be read as a live measurement.
#
# Exit codes: 0 ok; 2 bad flag; 3 no key; 4 the request failed (nothing recorded).
set -uo pipefail

MODEL="${SAMBANOVA_MODEL:-MiniMax-M2.7}"
URL="${SAMBANOVA_URL:-https://api.sambanova.ai/v1/chat/completions}"
PROMPT="Explain, in exactly three sentences, why a compiler that certifies its own placements matters for a dataflow chip."
MAX_TOKENS=256
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RECORD_PATH="${SAMBANOVA_RECORD_PATH:-$ROOT/benchmarks/results/sambacloud_baseline.json}"

usage() {
    cat <<'EOF'
Usage: scripts/accel/sambacloud-smoke.sh [--record] [--dry-run] [--help]

Streams one chat completion from SambaNova Cloud (OpenAI-compatible /v1/chat/completions, SSE) and prints one
JSON line: {"provider":"sambacloud","model":…,"ttft_s":…,"completion_tokens":…,"tokens_per_s":…,…}.

  --record    also write the line to benchmarks/results/sambacloud_baseline.json with an ISO `date`
              and provenance "measured_sambacloud_api" (--dry-run prints that exact shape)
  --dry-run   print the request shape with the key redacted; no network
  --help      this text

Offline: SAMBANOVA_STREAM_FILE=<captured SSE> replays the whole measuring path with no key and no network;
the record it writes is stamped provenance "replayed_sse_capture", never "measured_sambacloud_api".
SAMBANOVA_RECORD_PATH moves where --record writes.

Key: SAMBANOVA_API_KEY, else ~/.sambanova_key (never printed). Model: SAMBANOVA_MODEL (default MiniMax-M2.7).
EOF
}

DRY=0
RECORD=0
for a in "$@"; do
    case "$a" in
        --help|-h) usage; exit 0 ;;
        --dry-run) DRY=1 ;;
        --record) RECORD=1 ;;
        *) echo "sambacloud-smoke.sh: unknown option: $a" >&2; usage >&2; exit 2 ;;
    esac
done

BODY=$(python3 - "$MODEL" "$PROMPT" "$MAX_TOKENS" <<'PY'
import json, sys
model, prompt, max_tokens = sys.argv[1], sys.argv[2], int(sys.argv[3])
print(json.dumps({
    "model": model,
    "messages": [{"role": "user", "content": prompt}],
    "stream": True,
    "stream_options": {"include_usage": True},
    "max_tokens": max_tokens,
    "temperature": 0,
}))
PY
)

# `python3 -` would read its PROGRAM from stdin, which a heredoc then wins from the pipe — so the two
# filters live in files and stdin stays the stream they measure.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The measurement: the SSE stream on stdin → ONE JSON line. `SHAPE_ONLY=1` with nothing on stdin yields
# the SHAPE of that line (every measured field null), so what `--dry-run` shows can never drift from
# what is measured — it is the same program.
cat > "$WORK/measure.py" <<'PY'
import json, os, sys, time
from datetime import datetime, timezone
shape = bool(os.environ.get("SHAPE_ONLY"))
start = float(os.environ["START"])
first = last = None
chunks = 0
usage = None
error = None
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    if not line.startswith("data:"):
        # A non-SSE body is an error document from the API.
        try:
            error = json.loads(line)
        except ValueError:
            error = {"raw": line[:200]}
        continue
    now = time.time()
    payload = line[5:].strip()
    if payload == "[DONE]":
        break
    try:
        obj = json.loads(payload)
    except ValueError:
        continue
    if first is None:
        first = now
    last = now
    if obj.get("usage"):
        usage = obj["usage"]
    for ch in obj.get("choices", []):
        c = (ch.get("delta") or {}).get("content")
        if c:
            chunks += 1
tokens = (usage or {}).get("completion_tokens") or chunks
ttft = (first - start) if first else None
span = (last - first) if (first and last and last > first) else None
out = {
    "provider": "sambacloud",
    "endpoint": "chat/completions (stream)",
    "model": os.environ["MODEL"],
    "ttft_s": round(ttft, 4) if ttft else None,
    "completion_tokens": (tokens or None) if shape else tokens,
    "stream_span_s": round(span, 4) if span else None,
    "tokens_per_s": round(tokens / span, 2) if (span and tokens) else None,
    "token_source": None if shape else ("usage.completion_tokens" if usage else "content chunks"),
    "prompt_chars": len(os.environ["PROMPT"]),
    "measured_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "published_by": "shape only — no request was made" if shape else "us (one streamed request from this box; network latency included in TTFT)",
}
if os.environ.get("REPLAYED_FROM"):
    out["replayed_from"] = os.environ["REPLAYED_FROM"]
    out["published_by"] = "replayed from a captured SSE stream — NOT a live measurement"
if not shape and (error is not None or first is None):
    out["error"] = error or "no streamed chunk arrived"
print(json.dumps(out))
PY

# The record document: the measurement line, plus the ISO date and the provenance the ledger reads.
cat > "$WORK/record.py" <<'PY'
import json, sys
d = json.load(sys.stdin)
d["date"] = (d.get("measured_at") or "")[:10]
d["provenance"] = "replayed_sse_capture" if d.get("replayed_from") else "measured_sambacloud_api"
print(json.dumps(d, indent=2))
PY

measure() {
    START="${START:-0}" MODEL="$MODEL" PROMPT="$PROMPT" REPLAYED_FROM="${SAMBANOVA_STREAM_FILE:-}" python3 "$WORK/measure.py"
}

record_doc() {
    python3 "$WORK/record.py"
}

if [ "$DRY" = 1 ]; then
    echo "dry-run: the request that would be sent (key redacted):"
    echo "  POST $URL"
    echo "  Authorization: Bearer <redacted>"
    echo "  Content-Type: application/json"
    echo "  $BODY"
    echo "dry-run: TTFT = first 'data:' line; tok/s = completion_tokens (usage chunk, else content chunks) over the streamed span"
    echo "dry-run: --record would write this shape to $RECORD_PATH:"
    printf '' | SHAPE_ONLY=1 measure | record_doc
    exit 0
fi

# An offline replay of a captured SSE stream: the whole measuring path, no key, no network. The record it
# writes is stamped `replayed_sse_capture`, never `measured_sambacloud_api`.
REPLAY="${SAMBANOVA_STREAM_FILE:-}"
if [ -n "$REPLAY" ]; then
    if [ ! -r "$REPLAY" ]; then
        echo "sambacloud-smoke.sh: SAMBANOVA_STREAM_FILE is not readable: $REPLAY" >&2
        exit 4
    fi
    START="$(date +%s.%N)"
    RESULT=$(measure < "$REPLAY")
    echo "$RESULT"
    if [ "$RECORD" = 1 ]; then
        mkdir -p "$(dirname "$RECORD_PATH")"
        printf '%s' "$RESULT" | record_doc > "$RECORD_PATH"
        echo "recorded: $RECORD_PATH" >&2
    fi
    exit 0
fi

KEY="${SAMBANOVA_API_KEY:-}"
if [ -z "$KEY" ] && [ -f "$HOME/.sambanova_key" ]; then
    KEY="$(tr -d '[:space:]' < "$HOME/.sambanova_key")"
fi
if [ -z "$KEY" ]; then
    echo "sambacloud-smoke.sh: no key — set SAMBANOVA_API_KEY or put it in ~/.sambanova_key" >&2
    exit 3
fi

# curl reads the Authorization header from a config file descriptor: the key never appears in argv, and
# never passes through a print statement of ours either (a heredoc, straight into the fd curl reads).
config() {
    cat <<EOF
header = "Authorization: Bearer $KEY"
EOF
}

START="$(date +%s.%N)"
RESULT=$(curl -sS -N --max-time 180 --config <(config) -H "Content-Type: application/json" -d "$BODY" "$URL" 2>/dev/null | measure)

echo "$RESULT"
if printf '%s' "$RESULT" | grep -q '"error"'; then
    echo "sambacloud-smoke.sh: the request failed (see the JSON line); nothing recorded" >&2
    exit 4
fi
if [ "$RECORD" = 1 ]; then
    mkdir -p "$(dirname "$RECORD_PATH")"
    printf '%s' "$RESULT" | record_doc > "$RECORD_PATH"
    echo "recorded: $RECORD_PATH" >&2
fi
