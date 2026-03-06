import os, strutils

proc gcd(a_in, b_in: int64): int64 =
  var a = a_in
  var b = b_in
  while b > 0:
    let t = b
    b = a mod b
    a = t
  return a

let n = parseInt(paramStr(1))
var sum: int64 = 0
for i in 1..n:
  for j in i..n:
    sum += gcd(int64(i), int64(j))
echo sum
