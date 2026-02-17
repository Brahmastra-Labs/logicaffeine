import os, strutils

proc mergeSort(arr: var seq[int64]) =
  let n = arr.len
  if n < 2: return
  let mid = n div 2
  var left = arr[0..<mid]
  var right = arr[mid..<n]
  mergeSort(left)
  mergeSort(right)
  var i, j, k: int = 0
  while i < left.len and j < right.len:
    if left[i] <= right[j]: arr[k] = left[i]; i += 1
    else: arr[k] = right[j]; j += 1
    k += 1
  while i < left.len: arr[k] = left[i]; i += 1; k += 1
  while j < right.len: arr[k] = right[j]; j += 1; k += 1

let n = parseInt(paramStr(1))
var arr = newSeq[int64](n)
var seed: int64 = 42
for i in 0..<n:
  seed = (seed * 1103515245 + 12345) mod 2147483648
  arr[i] = (seed shr 16) and 0x7fff
mergeSort(arr)
var checksum: int64 = 0
for v in arr: checksum = (checksum + v) mod 1000000007
echo arr[0], " ", arr[^1], " ", checksum
