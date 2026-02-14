import os, strutils

proc sieve(limit: int): int =
  var isComposite = newSeq[bool](limit + 1)
  var count = 0
  for i in 2..limit:
    if not isComposite[i]:
      count += 1
      var j = i * i
      while j <= limit:
        isComposite[j] = true
        j += i
  return count

let limit = parseInt(paramStr(1))
echo sieve(limit)
