package main

import (
	"fmt"
	"os"
	"strconv"
)

func siftDown(arr []int64, start, end int) {
	root := start
	for 2*root+1 <= end {
		child := 2*root + 1
		swapIdx := root
		if arr[swapIdx] < arr[child] { swapIdx = child }
		if child+1 <= end && arr[swapIdx] < arr[child+1] { swapIdx = child + 1 }
		if swapIdx == root { return }
		arr[root], arr[swapIdx] = arr[swapIdx], arr[root]
		root = swapIdx
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
	for start := (n - 2) / 2; start >= 0; start-- { siftDown(arr, start, n-1) }
	for end := n - 1; end > 0; end-- {
		arr[0], arr[end] = arr[end], arr[0]
		siftDown(arr, 0, end-1)
	}
	checksum := int64(0)
	for _, v := range arr { checksum = (checksum + v) % 1000000007 }
	fmt.Printf("%d %d %d\n", arr[0], arr[n-1], checksum)
}
