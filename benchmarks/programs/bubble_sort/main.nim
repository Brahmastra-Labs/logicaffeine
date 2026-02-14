import os, strutils

let n = parseInt(paramStr(1))
var arr = newSeq[int32](n)
var seed: uint32 = 42
for i in 0..<n:
  seed = seed * 1103515245 + 12345
  arr[i] = int32((seed shr 16) and 0x7fff)
for i in 0..<(n - 1):
  for j in 0..<(n - 1 - i):
    if arr[j] > arr[j + 1]:
      swap(arr[j], arr[j + 1])
echo arr[0]
