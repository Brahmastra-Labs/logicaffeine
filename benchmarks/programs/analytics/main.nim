# analytics — filter → map → group-by → reduce pipeline (Nim port of main.c).
import os, strutils

const
  MOD: int64 = 1000000007
  NGROUP: int64 = 32768

proc main() =
  if paramCount() < 1:
    stderr.writeLine("usage: analytics <n>")
    quit(1)
  let n = parseBiggestInt(paramStr(1)).int64
  var seed: int64 = 42
  var groups: array[NGROUP, int64]
  var kept: int64 = 0
  for r in 0'i64 ..< n:
    seed = (seed * 1103515245 + 12345) mod 2147483648'i64
    let key = (seed shr 16) and 0x7fff
    seed = (seed * 1103515245 + 12345) mod 2147483648'i64
    let value = (seed shr 16) and 0x7fff
    if value mod 3 == 0: continue                   # FILTER
    let mapped = (value * key + 7) mod MOD           # MAP
    let g = key mod NGROUP                           # GROUP BY key
    groups[g] = (groups[g] + mapped) mod MOD         # aggregate SUM
    inc kept
  var checksum: int64 = 0                            # REDUCE
  for g in 0'i64 ..< NGROUP:
    checksum = (checksum * 31 + groups[g]) and 268435455
  echo kept, "\n", checksum

main()
