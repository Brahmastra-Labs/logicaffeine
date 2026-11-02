import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  var counts: array[1000, int64]
  var seed: int64 = 42
  for i in 0..<n:
    seed = (seed * 1103515245 + 12345) mod 2147483648
    counts[((seed shr 16) and 0x7fff) mod 1000] += 1
  var maxFreq, maxIdx: int64 = 0
  for i in 0..<1000:
    if counts[i] > maxFreq: maxFreq = counts[i]; maxIdx = int64(i)
  var h: int64 = 0
  for c in counts:
    h = (h * 31 + c) and 268435455
  echo maxFreq, " ", maxIdx, " ", h

main()
