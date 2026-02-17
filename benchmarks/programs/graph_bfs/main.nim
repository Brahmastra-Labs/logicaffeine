import os, strutils

const MAX_EDGES = 5

let n = parseInt(paramStr(1))
let primes = [31, 37, 41, 43, 47]
let offsets = [7, 13, 17, 23, 29]
var adj = newSeq[int](n * MAX_EDGES)
var adjCount = newSeq[int](n)
for p in 0..<MAX_EDGES:
  for i in 0..<n:
    let neighbor = (i * primes[p] + offsets[p]) mod n
    if neighbor != i:
      adj[i * MAX_EDGES + adjCount[i]] = neighbor
      adjCount[i] += 1
var queue = newSeq[int](n)
var dist = newSeq[int](n)
for i in 0..<n: dist[i] = -1
var front = 0
var back = 0
queue[back] = 0; back += 1
dist[0] = 0
while front < back:
  let v = queue[front]; front += 1
  for e in 0..<adjCount[v]:
    let u = adj[v * MAX_EDGES + e]
    if dist[u] == -1:
      dist[u] = dist[v] + 1
      queue[back] = u; back += 1
var reachable = 0
var totalDist = 0
for i in 0..<n:
  if dist[i] >= 0: reachable += 1; totalDist += dist[i]
echo reachable, " ", totalDist
