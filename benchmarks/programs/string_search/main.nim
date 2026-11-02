import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  var text = newStringOfCap(n)
  var seed: int64 = 42
  for pos in 0..<n:
    seed = (seed * 1103515245 + 12345) mod 2147483648
    text.add(chr(ord('a') + int((seed shr 16) mod 5)))
  let needle = "cabed"
  var count = 0
  var positions: seq[int] = @[]
  var i = 0
  while i <= text.len - needle.len:
    var matched = true
    for j in 0..<needle.len:
      if text[i + j] != needle[j]:
        matched = false
        break
    if matched:
      count += 1
      positions.add(i)
    i += 1
  var checksum = 0
  for p in positions: checksum = (checksum * 31 + p) and 268435455
  echo count, " ", checksum

main()
