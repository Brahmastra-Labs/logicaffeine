import os, strutils

let n = parseInt(paramStr(1))
var sum: int64 = 0
for i in 1..n:
  sum = (sum + i) mod 1000000007
echo sum
