import os, strutils

proc fib(n: int64): int64 =
  if n < 2: return n
  return fib(n - 1) + fib(n - 2)

let n = parseInt(paramStr(1)).int64
echo fib(n)
