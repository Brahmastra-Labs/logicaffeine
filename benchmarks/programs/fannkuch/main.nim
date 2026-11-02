import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  var perm1 = newSeq[int](n)
  for i in 0..<n: perm1[i] = i
  var perm = newSeq[int](n)
  var count = newSeq[int](n)
  var maxFlips, checksum, permCount: int = 0
  var r = n
  while true:
    while r > 1: count[r-1] = r; r -= 1
    for i in 0..<n: perm[i] = perm1[i]
    var flips = 0
    while perm[0] != 0:
      let k = perm[0] + 1
      for i in 0..<k div 2: swap(perm[i], perm[k-1-i])
      flips += 1
    if flips > maxFlips: maxFlips = flips
    if permCount mod 2 == 0: checksum += flips
    else: checksum -= flips
    permCount += 1
    while true:
      if r == n: echo checksum; echo maxFlips; return
      let p0 = perm1[0]
      for i in 0..<r: perm1[i] = perm1[i+1]
      perm1[r] = p0
      count[r] -= 1
      if count[r] > 0: break
      r += 1

main()
