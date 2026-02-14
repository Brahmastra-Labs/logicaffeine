import os, strutils, tables

let n = parseInt(paramStr(1))
var map = initTable[int, int]()
for i in 0..<n:
  map[i] = i * 2
var found = 0
for i in 0..<n:
  if map[i] == i * 2:
    found += 1
echo found
