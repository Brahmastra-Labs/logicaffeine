import os, strutils

proc sieve(limit: int): (int, int) =
  var isComposite = newSeq[bool](limit + 1)
  var count = 0
  var h = 0
  for i in 2..limit:
    h = (h * 31 + ord(not isComposite[i])) and 268435455
    if not isComposite[i]:
      count += 1
      var j = i * i
      while j <= limit:
        isComposite[j] = true
        j += i
  return (count, h)

proc main() =
  let limit = parseInt(paramStr(1))
  let (count, h) = sieve(limit)
  echo count, " ", h

main()
