import os, strutils, math, strformat
proc A(i, j: int): float64 = 1.0 / float64((i+j)*(i+j+1) div 2 + i + 1)
let n = parseInt(paramStr(1))
var u = newSeq[float64](n)
var v = newSeq[float64](n)
var t = newSeq[float64](n)
for i in 0..<n: u[i] = 1.0
for iter in 0..<10:
  for i in 0..<n: t[i] = 0; (for j in 0..<n: t[i] += A(i,j) * u[j])
  for i in 0..<n: v[i] = 0; (for j in 0..<n: v[i] += A(j,i) * t[j])
  for i in 0..<n: t[i] = 0; (for j in 0..<n: t[i] += A(i,j) * v[j])
  for i in 0..<n: u[i] = 0; (for j in 0..<n: u[i] += A(j,i) * t[j])
var vBv, vv: float64 = 0
for i in 0..<n: vBv += u[i]*v[i]; vv += v[i]*v[i]
echo &"{sqrt(vBv/vv):.9f}"
