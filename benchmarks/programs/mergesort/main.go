package main

import (
	"fmt"
	"os"
	"strconv"
)

func mergeSort(arr []int64) {
	n := len(arr)
	if n < 2 { return }
	mid := n / 2
	left := make([]int64, mid)
	right := make([]int64, n-mid)
	copy(left, arr[:mid])
	copy(right, arr[mid:])
	mergeSort(left)
	mergeSort(right)
	i, j, k := 0, 0, 0
	for i < mid && j < n-mid {
		if left[i] <= right[j] { arr[k] = left[i]; i++ } else { arr[k] = right[j]; j++ }
		k++
	}
	for i < mid { arr[k] = left[i]; i++; k++ }
	for j < n-mid { arr[k] = right[j]; j++; k++ }
}

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	arr := make([]int64, n)
	seed := int64(42)
	for i := 0; i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		arr[i] = (seed >> 16) & 0x7fff
	}
	mergeSort(arr)
	checksum := int64(0)
	for _, v := range arr { checksum = (checksum*31 + v) & 268435455 }
	fmt.Printf("%d %d %d\n", arr[0], arr[n-1], checksum)
}
