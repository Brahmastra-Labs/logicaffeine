import os, strutils

let n = parseInt(paramStr(1))
var arr = newSeq[int64](n)
var seed: int64 = 42
for i in 0..<n:
  seed = (seed * 1103515245 + 12345) mod 2147483648
  arr[i] = (seed shr 16) mod 1000
var counts: array[1000, int64]
for v in arr: counts[v] += 1
var sorted = newSeqOfCap[int64](n)
for v in 0..<1000:
  for c in 0..<counts[v]:
    sorted.add(int64(v))
var checksum: int64 = 0
for v in sorted: checksum = (checksum + v) mod 1000000007
echo sorted[0], " ", sorted[^1], " ", checksum
