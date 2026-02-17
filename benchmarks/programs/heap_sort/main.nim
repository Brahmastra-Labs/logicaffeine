import os, strutils

proc siftDown(arr: var seq[int64], start, stop: int) =
  var root = start
  while 2 * root + 1 <= stop:
    let child = 2 * root + 1
    var sw = root
    if arr[sw] < arr[child]: sw = child
    if child + 1 <= stop and arr[sw] < arr[child + 1]: sw = child + 1
    if sw == root: return
    swap(arr[root], arr[sw])
    root = sw

let n = parseInt(paramStr(1))
var arr = newSeq[int64](n)
var seed: int64 = 42
for i in 0..<n:
  seed = (seed * 1103515245 + 12345) mod 2147483648
  arr[i] = (seed shr 16) and 0x7fff
for s in countdown((n - 2) div 2, 0): siftDown(arr, s, n - 1)
for e in countdown(n - 1, 1):
  swap(arr[0], arr[e])
  siftDown(arr, 0, e - 1)
var checksum: int64 = 0
for v in arr: checksum = (checksum + v) mod 1000000007
echo arr[0], " ", arr[^1], " ", checksum
