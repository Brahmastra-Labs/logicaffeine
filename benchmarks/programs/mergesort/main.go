package main

import (
	"fmt"
	"os"
	"strconv"
)

func mergeSort(arr []int64) []int64 {
	n := len(arr)
	if n < 2 { return arr }
	mid := n / 2
	left := mergeSort(append([]int64{}, arr[:mid]...))
	right := mergeSort(append([]int64{}, arr[mid:]...))
	result := make([]int64, 0, n)
	i, j := 0, 0
	for i < len(left) && j < len(right) {
		if left[i] <= right[j] { result = append(result, left[i]); i++ } else { result = append(result, right[j]); j++ }
	}
	result = append(result, left[i:]...)
	result = append(result, right[j:]...)
	return result
}

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	arr := make([]int64, n)
	seed := int64(42)
	for i := 0; i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		arr[i] = (seed >> 16) & 0x7fff
	}
	arr = mergeSort(arr)
	checksum := int64(0)
	for _, v := range arr { checksum = (checksum + v) % 1000000007 }
	fmt.Printf("%d %d %d\n", arr[0], arr[n-1], checksum)
}
