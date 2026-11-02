import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  var arr = newSeq[int64](n)
  var seed: int64 = 42
  for i in 0..<n:
    seed = (seed * 1103515245 + 12345) mod 2147483648
    arr[i] = ((seed shr 16) and 0x7fff) mod 1000
  for i in 1..<n:
    arr[i] = (arr[i] + arr[i-1]) mod 1000000007
  var h: int64 = 0
  for v in arr:
    h = (h * 31 + v) and 268435455
  echo arr[n-1], " ", h

main()
