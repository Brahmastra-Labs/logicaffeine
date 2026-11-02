#!/usr/bin/env bash
# Regenerate the vendored NIST ACVP test-vector fixtures under
# crates/logicaffeine_tests/tests/data/acvp/ from the canonical source.
#
# Source: https://github.com/usnistgov/ACVP-Server (gen-val/json-files/) — a US-Government work,
# public domain. Pinned to a commit for reproducibility. The fixtures hold ONLY the ML-KEM-768 and
# ML-DSA-65 cases we gate against, with only the fields those gates consume, to stay compact.
#
# The gates (crates/logicaffeine_tests/tests/acvp_nist_vectors.rs for the LOGOS programs,
# crates/logicaffeine_tests/tests/pq_official_vectors.rs for the native kernels) run offline against the
# vendored fixtures; this script needs network and is run by hand when refreshing to a newer ACVP release.
set -euo pipefail

COMMIT="a7f283cdc87d2d6dd93c1bac59e5622c5f9f8324"   # ACVP-Server master @ 2026-07-31
BASE="https://raw.githubusercontent.com/usnistgov/ACVP-Server/${COMMIT}/gen-val/json-files"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/crates/logicaffeine_tests/tests/data/acvp"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$OUT"

echo "Fetching NIST ACVP vectors @ ${COMMIT:0:12} ..."
curl -fsSL "$BASE/ML-KEM-keyGen-FIPS203/internalProjection.json"     -o "$TMP/mlkem_keygen.json"
curl -fsSL "$BASE/ML-KEM-encapDecap-FIPS203/internalProjection.json" -o "$TMP/mlkem_encapdecap.json"
curl -fsSL "$BASE/ML-DSA-keyGen-FIPS204/internalProjection.json"     -o "$TMP/mldsa_keygen.json"
curl -fsSL "$BASE/ML-DSA-sigGen-FIPS204/internalProjection.json"     -o "$TMP/mldsa_siggen.json"
curl -fsSL "$BASE/ML-DSA-sigVer-FIPS204/internalProjection.json"     -o "$TMP/mldsa_sigver.json"

SRC="https://github.com/usnistgov/ACVP-Server (commit ${COMMIT}, 2026-07-31)" \
TMP="$TMP" OUT="$OUT" python3 - <<'PY'
import json, os
TMP, OUT, SRC = os.environ["TMP"], os.environ["OUT"], os.environ["SRC"]
NOTE = "Official NIST ACVP vectors (public domain). Regenerate with scripts/test/fetch-acvp-vectors.sh."
def groups(p): return json.load(open(f"{TMP}/{p}"))["testGroups"]
def dump(name, cases):
    json.dump({"source": SRC, "note": NOTE, "cases": cases}, open(f"{OUT}/{name}", "w"), indent=0)
    print(f"  {name}: {len(cases)} cases")

kem_kg = [{"tcId": t["tcId"], "d": t["d"], "ek": t["ek"]}
          for g in groups("mlkem_keygen.json") if g.get("parameterSet")=="ML-KEM-768" for t in g["tests"]]
kem_enc, kem_dec = [], []
for g in groups("mlkem_encapdecap.json"):
    if g.get("parameterSet")!="ML-KEM-768": continue
    if g.get("function")=="encapsulation":
        kem_enc += [{"tcId": t["tcId"], "ek": t["ek"], "m": t["m"], "c": t["c"], "k": t["k"]} for t in g["tests"]]
    elif g.get("function")=="decapsulation":
        kem_dec += [{"tcId": t["tcId"], "dk": t["dk"], "c": t["c"], "k": t["k"]} for t in g["tests"]]
dsa_kg = [{"tcId": t["tcId"], "seed": t["seed"], "pk": t["pk"]}
          for g in groups("mldsa_keygen.json") if g.get("parameterSet")=="ML-DSA-65" for t in g["tests"]]
dsa_sig = [{"tcId": t["tcId"], "sk": t["sk"], "message": t["message"], "signature": t["signature"]}
           for g in groups("mldsa_siggen.json")
           if (g.get("parameterSet")=="ML-DSA-65" and g.get("deterministic") is True
               and g.get("signatureInterface")=="external" and g.get("preHash")=="pure"
               and g.get("externalMu") is False)
           for t in g["tests"] if t.get("context","")==""]

# The external, pure interface under every context NIST draws (up to 255 bytes), deterministic and
# hedged (`rnd`), and its verification cases with their verdicts.
dsa_sig_ctx = [dict({"tcId": t["tcId"], "sk": t["sk"], "message": t["message"], "context": t["context"],
                     "signature": t["signature"]}, **({"rnd": t["rnd"]} if "rnd" in t else {}))
               for g in groups("mldsa_siggen.json")
               if (g.get("parameterSet")=="ML-DSA-65" and g.get("signatureInterface")=="external"
                   and g.get("preHash")=="pure" and g.get("externalMu") is False)
               for t in g["tests"]]
dsa_ver = [{"tcId": t["tcId"], "pk": t["pk"], "message": t["message"], "context": t["context"],
            "signature": t["signature"], "testPassed": t["testPassed"], "reason": t["reason"]}
           for g in groups("mldsa_sigver.json")
           if (g.get("parameterSet")=="ML-DSA-65" and g.get("signatureInterface")=="external"
               and g.get("preHash")=="pure" and g.get("externalMu") is False)
           for t in g["tests"]]
# FIPS 203 §7's input checks: the encapsulation-key modulus check and the decapsulation-key hash check.
kem_check = [dict({"tcId": t["tcId"], "function": g["function"], "testPassed": t["testPassed"],
                   "reason": t["reason"]}, **({"ek": t["ek"]} if g["function"]=="encapsulationKeyCheck" else {"dk": t["dk"]}))
             for g in groups("mlkem_encapdecap.json")
             if g.get("parameterSet")=="ML-KEM-768" and g.get("function") in ("encapsulationKeyCheck", "decapsulationKeyCheck")
             for t in g["tests"]]

dump("mlkem768_keygen.json", kem_kg)
dump("mlkem768_encaps.json", kem_enc)
dump("mlkem768_decaps.json", kem_dec)
dump("mldsa65_keygen.json",  dsa_kg)
dump("mldsa65_siggen.json",  dsa_sig)
dump("mldsa65_siggen_context.json", dsa_sig_ctx)
dump("mldsa65_sigver.json", dsa_ver)
dump("mlkem768_keycheck.json", kem_check)
PY
echo "Done -> $OUT"
