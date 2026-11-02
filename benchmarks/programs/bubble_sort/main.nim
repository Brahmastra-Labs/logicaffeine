import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  var arr = newSeq[int](n)
  var seed: uint32 = 42
  for i in 0..<n:
    seed = seed * 1103515245 + 12345
    arr[i] = int((seed shr 16) and 0x7fff)
  for i in 0..<(n - 1):
    for j in 0..<(n - 1 - i):
      if arr[j] > arr[j + 1]:
        swap(arr[j], arr[j + 1])
  var checksum = 0
  for v in arr: checksum = (checksum * 31 + v) and 268435455
  echo arr[0], " ", arr[^1], " ", checksum

main()
