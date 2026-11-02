# two_sum — the set main.c hand-rolls: flat 8-byte key slots, a power-of-two capacity at least twice n, Fibonacci
# hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory and the key
# 0 itself a flag beside the table.
import os, strutils

var table: seq[int64]
var mask: uint64
var shift: uint64
var hasZero = false

proc slotOf(key: int64): int {.inline.} =
  int((cast[uint64](key) * 0x9E3779B97F4A7C15'u64) shr shift)

proc contains(key: int64): bool =
  if key == 0:
    return hasZero
  var h = slotOf(key)
  while table[h] != 0:
    if table[h] == key:
      return true
    h = int((uint64(h) + 1) and mask)
  return false

proc insert(key: int64) =
  if key == 0:
    hasZero = true
    return
  var h = slotOf(key)
  while table[h] != 0:
    if table[h] == key:
      return
    h = int((uint64(h) + 1) and mask)
  table[h] = key

proc main() =
  let n = parseInt(paramStr(1))
  let target: int64 = 2147483647
  var arr = newSeq[int64](n)
  var seed: int64 = 42
  for i in 0..<n:
    seed = (seed * 1103515245 + 12345) mod 2147483648
    arr[i] = (if i > 0 and (seed shr 28) == 0: target - arr[seed mod int64(i)] else: seed)
  var bits = 4
  while (1 shl bits) < 2 * n:
    bits += 1
  let capacity = 1 shl bits
  mask = uint64(capacity - 1)
  shift = uint64(64 - bits)
  table = newSeq[int64](capacity)
  var count: int64 = 0
  for x in arr:
    if contains(target - x):
      count += 1
    insert(x)
  echo count

main()
