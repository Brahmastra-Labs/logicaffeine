import os, strutils

const MOD = 1000000007'i64

proc main() =
  let n = parseInt(paramStr(1))
  var a = newSeq[int64](n * n)
  var b = newSeq[int64](n * n)
  var c = newSeq[int64](n * n)
  var seed: int64 = 42
  for i in 0..<n * n:
    seed = (seed * 1103515245 + 12345) mod 2147483648
    a[i] = (seed shr 16) mod 100
  for i in 0..<n * n:
    seed = (seed * 1103515245 + 12345) mod 2147483648
    b[i] = (seed shr 16) mod 100
  for i in 0..<n:
    for k in 0..<n:
      for j in 0..<n:
        c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) mod MOD
  var checksum: int64 = 0
  for v in c:
    checksum = (checksum * 31 + v) and 268435455
  echo checksum

main()
