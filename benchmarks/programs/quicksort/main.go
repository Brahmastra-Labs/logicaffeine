package main

import (
	"fmt"
	"os"
	"strconv"
)

func partition(arr []int64, lo, hi int) int {
	pivot := arr[hi]
	i := lo
	for j := lo; j < hi; j++ {
		if arr[j] <= pivot { arr[i], arr[j] = arr[j], arr[i]; i++ }
	}
	arr[i], arr[hi] = arr[hi], arr[i]
	return i
}

func qs(arr []int64, lo, hi int) {
	if lo < hi {
		p := partition(arr, lo, hi)
		qs(arr, lo, p-1)
		qs(arr, p+1, hi)
	}
}

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	arr := make([]int64, n)
	seed := int64(42)
	for i := 0; i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		arr[i] = (seed >> 16) & 0x7fff
	}
	qs(arr, 0, n-1)
	checksum := int64(0)
	for _, v := range arr { checksum = (checksum + v) % 1000000007 }
	fmt.Printf("%d %d %d\n", arr[0], arr[n-1], checksum)
}
