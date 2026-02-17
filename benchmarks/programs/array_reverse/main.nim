import os, strutils

let n = parseInt(paramStr(1))
var arr = newSeq[int64](n)
var seed: int64 = 42
for i in 0..<n:
  seed = (seed * 1103515245 + 12345) mod 2147483648
  arr[i] = (seed shr 16) and 0x7fff
var lo = 0
var hi = n - 1
while lo < hi:
  swap(arr[lo], arr[hi])
  lo += 1; hi -= 1
echo arr[0], " ", arr[n-1], " ", arr[n div 2]
