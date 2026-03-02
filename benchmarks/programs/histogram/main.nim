import os, strutils

let n = parseInt(paramStr(1))
var counts: array[1000, int64]
var seed: int64 = 42
for i in 0..<n:
  seed = (seed * 1103515245 + 12345) mod 2147483648
  counts[((seed shr 16) and 0x7fff) mod 1000] += 1
var maxFreq, maxIdx, distinctCount: int64 = 0
for i in 0..<1000:
  if counts[i] > 0: distinctCount += 1
  if counts[i] > maxFreq: maxFreq = counts[i]; maxIdx = int64(i)
echo maxFreq, " ", maxIdx, " ", distinctCount
