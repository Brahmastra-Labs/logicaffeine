import os, strutils, math, strformat
proc A(i, j: int): float64 = 1.0 / float64((i+j)*(i+j+1) div 2 + i + 1)
proc main() =
  let n = parseInt(paramStr(1))
  var u = newSeq[float64](n)
  var v = newSeq[float64](n)
  var t = newSeq[float64](n)
  for i in 0..<n: u[i] = 1.0
  for iter in 0..<10:
    for i in 0..<n:
      var s = 0.0
      for j in 0..<n: s += A(i,j) * u[j]
      t[i] = s
    for i in 0..<n:
      var s = 0.0
      for j in 0..<n: s += A(j,i) * t[j]
      v[i] = s
    for i in 0..<n:
      var s = 0.0
      for j in 0..<n: s += A(i,j) * v[j]
      t[i] = s
    for i in 0..<n:
      var s = 0.0
      for j in 0..<n: s += A(j,i) * t[j]
      u[i] = s
  var vBv, vv: float64 = 0
  for i in 0..<n: vBv += u[i]*v[i]; vv += v[i]*v[i]
  echo &"{sqrt(vBv/vv):.9f}"

main()
