# LOGOS Standard-Library Parity Ledger

_Generated from catalog/superset.json by scripts/bench/gen-parity-ledger.py — do not edit by hand._

The stdlib is pure LOGOS and must be FASTER than every C/Python/Rust/Go counterpart.
A capability is fully landed only when it is correct on all tiers AND benchmarked faster.

- **Correctness parity:** 100.0% (548 green / 548 in-scope; 6 n/a excluded)
- **Performance parity:** 7.3% (38 faster / 520 perf-gated)
- **Upstream triage:** 25 in-scope backlog · 11438 N/A-by-design · 1160 claimed (of 12623)

| Domain | Green | In-scope | Correct % | Faster | Perf-gated | Faster % |
|--------|-------|----------|-----------|--------|------------|----------|
| bignum | 6 | 6 | 100.0% | 0 | 6 | 0.0% |
| bits | 12 | 12 | 100.0% | 5 | 12 | 41.7% |
| builder | 15 | 15 | 100.0% | 0 | 15 | 0.0% |
| bytes | 37 | 37 | 100.0% | 0 | 37 | 0.0% |
| c_control | 0 | 0 | n/a | 0 | 0 | n/a |
| c_memory | 0 | 0 | n/a | 0 | 0 | n/a |
| calendar | 8 | 8 | 100.0% | 0 | 8 | 0.0% |
| chars | 17 | 17 | 100.0% | 4 | 17 | 23.5% |
| collections | 11 | 11 | 100.0% | 0 | 11 | 0.0% |
| complex | 32 | 32 | 100.0% | 0 | 20 | 0.0% |
| compress | 3 | 3 | 100.0% | 0 | 3 | 0.0% |
| concurrency | 0 | 0 | n/a | 0 | 0 | n/a |
| convert | 22 | 22 | 100.0% | 1 | 22 | 4.5% |
| decimal | 2 | 2 | 100.0% | 0 | 2 | 0.0% |
| encoding | 33 | 33 | 100.0% | 0 | 33 | 0.0% |
| env | 1 | 1 | 100.0% | 0 | 0 | n/a |
| error | 3 | 3 | 100.0% | 0 | 0 | n/a |
| heap | 15 | 15 | 100.0% | 0 | 15 | 0.0% |
| io | 2 | 2 | 100.0% | 0 | 0 | n/a |
| iter | 57 | 57 | 100.0% | 1 | 57 | 1.8% |
| json | 1 | 1 | 100.0% | 0 | 1 | 0.0% |
| math | 96 | 96 | 100.0% | 18 | 93 | 19.4% |
| operator | 23 | 23 | 100.0% | 4 | 18 | 22.2% |
| random | 20 | 20 | 100.0% | 0 | 20 | 0.0% |
| regex | 18 | 18 | 100.0% | 0 | 18 | 0.0% |
| sort | 2 | 2 | 100.0% | 0 | 2 | 0.0% |
| statistics | 16 | 16 | 100.0% | 1 | 16 | 6.2% |
| text | 86 | 86 | 100.0% | 4 | 86 | 4.7% |
| time | 10 | 10 | 100.0% | 0 | 8 | 0.0% |

## Provenance

Pinned upstream sources (see sources.lock.toml): C = C23 (WG14 N3220) / glibc 2.39, Python = 3.12.3, Rust = 1.96.0, Go = go1.22.2, Java = curated (secondary).

