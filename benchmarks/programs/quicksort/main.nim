import os, strutils

proc partition(arr: var seq[int64], lo, hi: int): int =
  let pivot = arr[hi]
  var i = lo
  for j in lo..<hi:
    if arr[j] <= pivot:
      swap(arr[i], arr[j]); i += 1
  swap(arr[i], arr[hi])
  return i

proc qs(arr: var seq[int64], lo, hi: int) =
  if lo < hi:
    let p = partition(arr, lo, hi)
    qs(arr, lo, p - 1)
    qs(arr, p + 1, hi)

let n = parseInt(paramStr(1))
var arr = newSeq[int64](n)
var seed: int64 = 42
for i in 0..<n:
  seed = (seed * 1103515245 + 12345) mod 2147483648
  arr[i] = (seed shr 16) and 0x7fff
qs(arr, 0, n - 1)
var checksum: int64 = 0
for v in arr: checksum = (checksum + v) mod 1000000007
echo arr[0], " ", arr[^1], " ", checksum
