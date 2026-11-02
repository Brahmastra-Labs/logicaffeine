# collect — the table main.c hand-rolls: flat 16-byte (key, value) slots, a power-of-two capacity at least twice n,
# Fibonacci hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory
# and the key 0 itself kept beside the table.
import os, strutils

type Slot = object
  key, value: int

var table: seq[Slot]
var mask: uint64
var shift: uint64
var hasZero = false
var zeroValue = 0

proc slotOf(key: int): int {.inline.} =
  int((cast[uint64](key) * 0x9E3779B97F4A7C15'u64) shr shift)

proc insert(key, value: int) =
  if key == 0:
    hasZero = true
    zeroValue = value
    return
  var i = slotOf(key)
  while table[i].key != 0 and table[i].key != key:
    i = int((uint64(i) + 1) and mask)
  table[i] = Slot(key: key, value: value)

# The slot holding `key`, -1 when it holds none, -2 for the key 0.
proc find(key: int): int =
  if key == 0:
    return (if hasZero: -2 else: -1)
  var i = slotOf(key)
  while table[i].key != 0:
    if table[i].key == key:
      return i
    i = int((uint64(i) + 1) and mask)
  return -1

proc valueAt(slot: int): int =
  if slot == -2: zeroValue else: table[slot].value

proc main() =
  let n = parseInt(paramStr(1))
  var bits = 4
  while (1 shl bits) < 2 * n:
    bits += 1
  let capacity = 1 shl bits
  mask = uint64(capacity - 1)
  shift = uint64(64 - bits)
  table = newSeq[Slot](capacity)
  var seed = 42
  for i in 0..<n:
    seed = (seed * 1103515245 + 12345) mod 2147483648
    insert(seed, seed * 2)
  var hit = 42
  var miss = seed
  var found = 0
  var h = 0
  for i in 0..<n:
    hit = (hit * 1103515245 + 12345) mod 2147483648
    var s = find(hit)
    if s != -1:
      found += 1
      h = (h * 31 + valueAt(s)) and 268435455
    miss = (miss * 1103515245 + 12345) mod 2147483648
    s = find(miss)
    if s != -1:
      found += 1
      h = (h * 31 + valueAt(s)) and 268435455
  echo found, " ", h

main()
