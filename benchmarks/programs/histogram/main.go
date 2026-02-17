package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	counts := make([]int64, 1000)
	seed := int64(42)
	for i := int64(0); i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		counts[((seed>>16)&0x7fff)%1000]++
	}
	var maxFreq, maxIdx, distinct int64
	for i := 0; i < 1000; i++ {
		if counts[i] > 0 { distinct++ }
		if counts[i] > maxFreq { maxFreq = counts[i]; maxIdx = int64(i) }
	}
	fmt.Printf("%d %d %d\n", maxFreq, maxIdx, distinct)
}
