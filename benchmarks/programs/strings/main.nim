import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  var result = newStringOfCap(n * 6)
  for i in 0..<n:
    result.addInt(i)
    result.add(' ')
  var spaces = 0
  for c in result:
    if c == ' ':
      spaces += 1
  var h = 0
  for c in result:
    h = (h * 31 + ord(c)) and 268435455
  echo spaces, " ", h

main()
