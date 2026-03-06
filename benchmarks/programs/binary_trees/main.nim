import os, strutils
proc makeCheck(d: int): int64 =
  if d == 0: return 1
  return 1 + makeCheck(d-1) + makeCheck(d-1)
let n = parseInt(paramStr(1))
let mn = 4; let mx = max(mn + 2, n)
echo "stretch tree of depth ", mx+1, "\t check: ", makeCheck(mx+1)
let ll = makeCheck(mx)
var d = mn
while d <= mx:
  let it = 1 shl (mx - d + mn)
  var tc: int64 = 0
  for i in 0..<it: tc += makeCheck(d)
  echo it, "\t trees of depth ", d, "\t check: ", tc
  d += 2
echo "long lived tree of depth ", mx, "\t check: ", ll
