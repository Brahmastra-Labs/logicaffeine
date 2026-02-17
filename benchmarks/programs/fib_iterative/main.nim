import os, strutils

let n = parseInt(paramStr(1))
var a: int64 = 0
var b: int64 = 1
for i in 0..<n:
  let temp = b
  b = (a + b) mod 1000000007
  a = temp
echo a
