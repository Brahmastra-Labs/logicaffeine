import os, strutils

let n = parseInt(paramStr(1))
var total: int64 = 0
for i in 1..n:
  var k = int64(i)
  while k != 1:
    if k mod 2 == 0: k = k div 2
    else: k = 3 * k + 1
    total += 1
echo total
