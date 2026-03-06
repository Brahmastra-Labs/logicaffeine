import os, strutils
let n = parseInt(paramStr(1))
var count = 0
for y in 0..<n:
  for x in 0..<n:
    let cr = 2.0 * float64(x) / float64(n) - 1.5
    let ci = 2.0 * float64(y) / float64(n) - 1.0
    var zr, zi: float64 = 0.0
    var inside = true
    for i in 0..<50:
      let t = zr*zr - zi*zi + cr
      zi = 2.0*zr*zi + ci; zr = t
      if zr*zr + zi*zi > 4.0: inside = false; break
    if inside: count += 1
echo count
