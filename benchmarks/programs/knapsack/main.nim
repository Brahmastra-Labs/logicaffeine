import os, strutils
let n = parseInt(paramStr(1))
let capacity = n * 5
var prev = newSeq[int64](capacity + 1)
var curr = newSeq[int64](capacity + 1)
for i in 0..<n:
  let w = (i * 17 + 3) mod 50 + 1
  let v = int64((i * 31 + 7) mod 100 + 1)
  for j in 0..capacity:
    curr[j] = prev[j]
    if j >= w and prev[j - w] + v > curr[j]: curr[j] = prev[j - w] + v
  swap(prev, curr)
echo prev[capacity]
