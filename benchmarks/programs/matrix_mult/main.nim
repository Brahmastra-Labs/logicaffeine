import os, strutils

const MOD = 1000000007'i64

let n = parseInt(paramStr(1))
var a = newSeq[int64](n * n)
var b = newSeq[int64](n * n)
var c = newSeq[int64](n * n)
for i in 0..<n:
  for j in 0..<n:
    a[i * n + j] = int64((i * n + j) mod 100)
    b[i * n + j] = int64((j * n + i) mod 100)
for i in 0..<n:
  for k in 0..<n:
    for j in 0..<n:
      c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) mod MOD
var checksum: int64 = 0
for v in c:
  checksum = (checksum + v) mod MOD
echo checksum
