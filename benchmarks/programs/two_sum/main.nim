import os, strutils, sets
let n = parseInt(paramStr(1))
var arr = newSeq[int64](n)
var seed: int64 = 42
for i in 0..<n: seed = (seed * 1103515245 + 12345) mod 2147483648; arr[i] = ((seed shr 16) and 0x7fff) mod int64(n)
var seen = initHashSet[int64]()
var count: int64 = 0
for x in arr:
  let c = int64(n) - x
  if c >= 0 and c in seen: count += 1
  seen.incl(x)
echo count
