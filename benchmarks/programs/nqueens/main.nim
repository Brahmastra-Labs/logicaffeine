import os, strutils

proc solve(row, cols, diag1, diag2, n: int): int =
  if row == n: return 1
  var available = ((1 shl n) - 1) and not (cols or diag1 or diag2)
  while available != 0:
    let bit = available and (-available)
    available = available xor bit
    result += solve(row + 1, cols or bit, (diag1 or bit) shl 1, (diag2 or bit) shr 1, n)

let n = parseInt(paramStr(1))
echo solve(0, 0, 0, 0, n)
