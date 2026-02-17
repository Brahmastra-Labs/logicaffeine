import os, strutils

let n = parseInt(paramStr(1))
var count: int64 = 0
for i in 2..n:
  var isPrime = true
  var d = 2
  while d * d <= i:
    if i mod d == 0:
      isPrime = false
      break
    d += 1
  if isPrime: count += 1
echo count
