import os, strutils

proc main() =
  let n = parseInt(paramStr(1))
  let coins = [1, 5, 10, 25, 50, 100]
  var dp = newSeq[int64](n + 1)
  dp[0] = 1
  for c in coins:
    for j in c..n: dp[j] = (dp[j] + dp[j - c]) mod 1000000007
  var checksum: int64 = 0
  for v in dp: checksum = (checksum * 31 + v) and 268435455
  echo dp[n], " ", checksum

main()
