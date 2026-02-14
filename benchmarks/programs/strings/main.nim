import os, strutils

let n = parseInt(paramStr(1))
var result = ""
for i in 0..<n:
  result.add($i)
  result.add(' ')
var spaces = 0
for c in result:
  if c == ' ':
    spaces += 1
echo spaces
