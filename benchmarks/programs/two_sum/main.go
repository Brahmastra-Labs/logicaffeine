// two_sum — the set main.c hand-rolls: flat 8-byte key slots, a power-of-two capacity at least twice n, Fibonacci
// hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory and the key
// 0 itself a flag beside the table.
package main

import (
	"fmt"
	"os"
	"strconv"
)

var (
	table   []int64
	mask    uint64
	shift   uint
	hasZero bool
)

func slotOf(key int64) uint64 {
	return (uint64(key) * 0x9E3779B97F4A7C15) >> shift
}

func contains(key int64) bool {
	if key == 0 {
		return hasZero
	}
	h := slotOf(key)
	for table[h] != 0 {
		if table[h] == key {
			return true
		}
		h = (h + 1) & mask
	}
	return false
}

func insert(key int64) {
	if key == 0 {
		hasZero = true
		return
	}
	h := slotOf(key)
	for table[h] != 0 {
		if table[h] == key {
			return
		}
		h = (h + 1) & mask
	}
	table[h] = key
}

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	target := int64(2147483647)
	arr := make([]int64, n)
	seed := int64(42)
	for i := int64(0); i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		if i > 0 && seed>>28 == 0 {
			arr[i] = target - arr[seed%i]
		} else {
			arr[i] = seed
		}
	}
	bits := uint(4)
	for (int64(1) << bits) < 2*n {
		bits++
	}
	capacity := uint64(1) << bits
	mask = capacity - 1
	shift = 64 - bits
	table = make([]int64, capacity)
	count := int64(0)
	for _, x := range arr {
		if contains(target - x) {
			count++
		}
		insert(x)
	}
	fmt.Println(count)
}
