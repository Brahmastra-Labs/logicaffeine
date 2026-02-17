import os, strutils, algorithm
let n = parseInt(paramStr(1))
var perm1 = newSeq[int](n)
for i in 0..<n: perm1[i] = i
var count = newSeq[int](n)
var maxFlips, checksum, permCount: int = 0
var r = n
while true:
  while r > 1: count[r-1] = r; r -= 1
  var perm = perm1
  var flips = 0
  while perm[0] != 0:
    let k = perm[0] + 1
    reverse(perm, 0, k - 1)
    flips += 1
  if flips > maxFlips: maxFlips = flips
  if permCount mod 2 == 0: checksum += flips
  else: checksum -= flips
  permCount += 1
  while true:
    if r == n: echo checksum; echo maxFlips; quit(0)
    let p0 = perm1[0]
    for i in 0..<r: perm1[i] = perm1[i+1]
    perm1[r] = p0
    count[r] -= 1
    if count[r] > 0: break
    r += 1
