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
		arr[i] = (seed >> 16) % 1000
	}
	counts := make([]int64, 1000)
	for _, v := range arr { counts[v]++ }
	sorted := make([]int64, 0, n)
	for v := 0; v < 1000; v++ {
		for c := int64(0); c < counts[v]; c++ {
			sorted = append(sorted, int64(v))
		}
	}
	checksum := int64(0)
	for _, v := range sorted { checksum = (checksum + v) % 1000000007 }
	fmt.Printf("%d %d %d\n", sorted[0], sorted[n-1], checksum)
}
