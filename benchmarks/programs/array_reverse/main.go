package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	arr := make([]int64, n)
	seed := int64(42)
	for i := 0; i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		arr[i] = (seed >> 16) & 0x7fff
	}
	lo, hi := 0, n-1
	for lo < hi {
		arr[lo], arr[hi] = arr[hi], arr[lo]
		lo++; hi--
	}
	fmt.Printf("%d %d %d\n", arr[0], arr[n-1], arr[n/2])
}
