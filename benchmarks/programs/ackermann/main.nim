import os, strutils

proc ackermann(m, n: int64): int64 =
  if m == 0: return n + 1
  if n == 0: return ackermann(m - 1, 1)
  return ackermann(m - 1, ackermann(m, n - 1))

let m = parseInt(paramStr(1)).int64
echo ackermann(3, m)
