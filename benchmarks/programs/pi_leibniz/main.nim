import os, strutils, strformat
let n = parseInt(paramStr(1))
var sum, sign: float64 = 0.0
sign = 1.0
for k in 0..<n:
  sum += sign / (2.0 * float64(k) + 1.0)
  sign = -sign
echo &"{sum * 4.0:.15f}"
