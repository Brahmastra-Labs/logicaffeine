import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  var arr = newSeq[int64](n)
  for i in 0..<n:
    arr[i] = (int64(i) * 7 + 3) mod 1000000
  var h: int64 = 0
  for v in arr:
    h = (h * 31 + v) and 268435455
  echo h

main()
