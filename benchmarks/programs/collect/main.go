// collect — the table main.c hand-rolls: flat 16-byte (key, value) slots, a power-of-two capacity at least twice n,
// Fibonacci hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory
// and the key 0 itself kept beside the table.
package main

import (
	"fmt"
	"os"
	"strconv"
)

type slot struct{ key, value int64 }

var (
	table     []slot
	mask      uint64
	shift     uint
	hasZero   bool
	zeroValue int64
)

func slotOf(key int64) uint64 {
	return (uint64(key) * 0x9E3779B97F4A7C15) >> shift
}

func insert(key, value int64) {
	if key == 0 {
		hasZero = true
		zeroValue = value
		return
	}
	i := slotOf(key)
	for table[i].key != 0 && table[i].key != key {
		i = (i + 1) & mask
	}
	table[i] = slot{key, value}
}

func find(key int64) (int64, bool) {
	if key == 0 {
		return zeroValue, hasZero
	}
	i := slotOf(key)
	for table[i].key != 0 {
		if table[i].key == key {
			return table[i].value, true
		}
		i = (i + 1) & mask
	}
	return 0, false
}

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	bits := uint(4)
	for (int64(1) << bits) < 2*n {
		bits++
	}
	capacity := uint64(1) << bits
	mask = capacity - 1
	shift = 64 - bits
	table = make([]slot, capacity)
	seed := int64(42)
	for i := int64(0); i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		insert(seed, seed*2)
	}
	hit, miss := int64(42), seed
	var found, h int64
	for i := int64(0); i < n; i++ {
		hit = (hit*1103515245 + 12345) % 2147483648
		if v, ok := find(hit); ok {
			found++
			h = (h*31 + v) & 268435455
		}
		miss = (miss*1103515245 + 12345) % 2147483648
		if v, ok := find(miss); ok {
			found++
			h = (h*31 + v) & 268435455
		}
	}
	fmt.Println(found, h)
}
