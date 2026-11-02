// analytics — filter → map → group-by → reduce pipeline (Go port of main.c).
package main

import (
	"fmt"
	"os"
	"strconv"
)

const (
	MOD    int64 = 1000000007
	NGROUP int64 = 32768
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, "usage: analytics <n>")
		os.Exit(1)
	}
	n, err := strconv.ParseInt(os.Args[1], 10, 64)
	if err != nil {
		fmt.Fprintln(os.Stderr, "usage: analytics <n>")
		os.Exit(1)
	}
	var seed int64 = 42
	var groups [NGROUP]int64
	var kept int64 = 0
	for r := int64(0); r < n; r++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		key := (seed >> 16) & 0x7fff
		seed = (seed*1103515245 + 12345) % 2147483648
		value := (seed >> 16) & 0x7fff
		if value%3 == 0 { // FILTER
			continue
		}
		mapped := (value*key + 7) % MOD    // MAP
		g := key % NGROUP                  // GROUP BY key
		groups[g] = (groups[g] + mapped) % MOD // aggregate SUM
		kept++
	}
	var checksum int64 = 0 // REDUCE
	for g := int64(0); g < NGROUP; g++ {
		checksum = (checksum*31 + groups[g]) & 268435455
	}
	fmt.Printf("%d\n%d\n", kept, checksum)
}
