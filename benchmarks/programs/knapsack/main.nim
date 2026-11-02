import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  let capacity = n * 5
  var weight = newSeq[int64](n)
  var value = newSeq[int64](n)
  var seed: int64 = 42
  for i in 0..<n:
    seed = (seed * 1103515245 + 12345) mod 2147483648
    weight[i] = (seed shr 16) mod 50 + 1
    seed = (seed * 1103515245 + 12345) mod 2147483648
    value[i] = (seed shr 16) mod 100 + 1
  var prev = newSeq[int64](capacity + 1)
  var curr = newSeq[int64](capacity + 1)
  for i in 0..<n:
    let w = int(weight[i])
    let v = value[i]
    for j in 0..capacity:
      curr[j] = prev[j]
      if j >= w and prev[j - w] + v > curr[j]: curr[j] = prev[j - w] + v
    swap(prev, curr)
  var h: int64 = 0
  for j in 0..capacity:
    h = (h * 31 + prev[j]) and 268435455
  echo prev[capacity], " ", h

main()
