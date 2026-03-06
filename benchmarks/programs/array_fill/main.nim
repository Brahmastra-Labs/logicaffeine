import os, strutils

let n = parseInt(paramStr(1))
var arr = newSeq[int64](n)
for i in 0..<n:
  arr[i] = (int64(i) * 7 + 3) mod 1000000
var sum: int64 = 0
for v in arr:
  sum = (sum + v) mod 1000000007
echo sum
