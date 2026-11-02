#!/usr/bin/env python3
"""emit_wpe_soc_stand_in — the `emit_wpe_soc` command line as `scripts/logos-chip.py` drives it, answering from the
lock's cases (`E2E_CASES`) instead of compiling: a case's program gets its image written to `--wire` and its facts on
stdout (`golden=` or `golden_json=`, `lifter=`, `profile=`, `bytes=`, and `map_json=` when the case carries the
fabric map the lock computed); any other program is refused on stderr with
exit status 1, the way the real emitter refuses a program that does not fit Board B's engine. Every command line it
is given is appended to `emit_calls.txt` beside the cases (the host runs it from the repo, which it never writes)."""
import json
import os
import sys

args = sys.argv[1:]
with open(os.path.join(os.path.dirname(os.path.abspath(os.environ["E2E_CASES"])), "emit_calls.txt"), "a") as log:
    log.write(" ".join(args) + "\n")
opt = {}
i = 0
while i < len(args):
    if args[i] == "--main":
        opt["main"] = True
        i += 1
    else:
        opt[args[i].lstrip("-")] = args[i + 1]
        i += 2
for case in json.load(open(os.environ["E2E_CASES"])):
    if os.path.abspath(case["source"]) == os.path.abspath(opt["program"]):
        image = open(case["wire"], "rb").read()
        open(opt["wire"], "wb").write(image)
        golden = case["golden"]
        print(f"golden_json={json.dumps(golden)}" if isinstance(golden, list) else f"golden={golden}")
        print(f"lifter={'main' if opt.get('main') else 'cfg'}")
        print(f"profile={opt['profile']}")
        print(f"bytes={len(image)}")
        if case.get("map") is not None:
            print(f"map_json={json.dumps(case['map'])}")
        sys.exit(0)
name = os.path.basename(opt["program"])
sys.stderr.write(f"emit_wpe_soc: {name} does not fit the {opt['profile']} engine: 9 blocks > hw MAXBLK=8\n")
sys.exit(1)
