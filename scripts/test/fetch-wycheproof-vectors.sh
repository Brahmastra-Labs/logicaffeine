#!/usr/bin/env bash
# Regenerate the vendored Project Wycheproof fixtures under
# crates/logicaffeine_tests/tests/data/wycheproof/ from the canonical source.
#
# Source: https://github.com/C2SP/wycheproof (testvectors_v1/), Apache-2.0. Pinned to a commit for
# reproducibility. The fixtures hold the ML-DSA-65, ML-KEM-768, ChaCha20-Poly1305, XChaCha20-Poly1305,
# HMAC-SHA256 and PBKDF2-HMAC-SHA256 files with only the fields the gates consume (the DER and PKCS#8
# encodings are dropped), to stay compact.
#
# The gates (crates/logicaffeine_tests/tests/pq_official_vectors.rs, aead_official_vectors.rs) run
# offline against the vendored fixtures; this script needs network and is run by hand when refreshing
# to a newer Wycheproof.
set -euo pipefail

COMMIT="d17cc89fa5c8f4f4d378b5385ebf75da5d0926c9"   # C2SP/wycheproof main @ 2026-10-23
BASE="https://raw.githubusercontent.com/C2SP/wycheproof/${COMMIT}/testvectors_v1"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="$ROOT/crates/logicaffeine_tests/tests/data/wycheproof"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$OUT"

echo "Fetching Wycheproof vectors @ ${COMMIT:0:12} ..."
for f in mldsa_65_verify_test mldsa_65_sign_seed_test mldsa_65_sign_noseed_test \
         mlkem_768_test mlkem_768_encaps_test mlkem_768_semi_expanded_decaps_test \
         chacha20_poly1305_test xchacha20_poly1305_test hmac_sha256_test pbkdf2_hmacsha256_test; do
  curl -fsSL "$BASE/$f.json" -o "$TMP/$f.json"
done

SRC="https://github.com/C2SP/wycheproof (commit ${COMMIT}, 2026-10-23)" \
TMP="$TMP" OUT="$OUT" python3 - <<'PY'
import json, os
TMP, OUT, SRC = os.environ["TMP"], os.environ["OUT"], os.environ["SRC"]
NOTE = ("Project Wycheproof test vectors (Apache-2.0), trimmed to the fields the gate reads. "
        "Regenerate with scripts/test/fetch-wycheproof-vectors.sh.")
TEST_FIELDS = ("tcId", "comment", "flags", "result", "seed", "msg", "ctx", "mu", "rnd", "sig",
               "m", "ek", "dk", "c", "K", "key", "iv", "aad", "ct", "tag", "password", "salt",
               "iterationCount", "dkLen")
GROUP_FIELDS = ("type", "parameterSet", "publicKey", "privateKey", "privateSeed", "keySize", "ivSize",
                "tagSize")

def trim(name, out):
    d = json.load(open(f"{TMP}/{name}.json"))
    groups = []
    for g in d["testGroups"]:
        group = {k: g[k] for k in GROUP_FIELDS if k in g}
        group["tests"] = [{k: t[k] for k in TEST_FIELDS if k in t} for t in g["tests"]]
        groups.append(group)
    count = sum(len(g["tests"]) for g in groups)
    assert count == d["numberOfTests"], (name, count, d["numberOfTests"])
    json.dump({"source": SRC, "note": NOTE, "algorithm": d["algorithm"], "numberOfTests": count,
               "testGroups": groups}, open(f"{OUT}/{out}", "w"), indent=0)
    print(f"  {out}: {count} cases")

trim("mldsa_65_verify_test", "mldsa65_verify.json")
trim("mldsa_65_sign_seed_test", "mldsa65_sign_seed.json")
trim("mldsa_65_sign_noseed_test", "mldsa65_sign_noseed.json")
trim("mlkem_768_test", "mlkem768_decaps.json")
trim("mlkem_768_encaps_test", "mlkem768_encaps.json")
trim("mlkem_768_semi_expanded_decaps_test", "mlkem768_semi_expanded_decaps.json")
trim("chacha20_poly1305_test", "chacha20_poly1305.json")
trim("xchacha20_poly1305_test", "xchacha20_poly1305.json")
trim("hmac_sha256_test", "hmac_sha256.json")
trim("pbkdf2_hmacsha256_test", "pbkdf2_hmac_sha256.json")
PY
echo "Done -> $OUT"
