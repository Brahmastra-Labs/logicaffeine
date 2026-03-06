import os, strutils

let n = parseInt(paramStr(1))
var text = newStringOfCap(n)
var pos = 0
while pos < n:
  if pos > 0 and pos mod 1000 == 0 and pos + 5 <= n:
    text.add("XXXXX")
    pos += 5
  else:
    text.add(chr(ord('a') + pos mod 5))
    pos += 1
let needle = "XXXXX"
var count = 0
var i = 0
while i <= text.len - needle.len:
  if text[i..<(i+needle.len)] == needle:
    count += 1
  i += 1
echo count
