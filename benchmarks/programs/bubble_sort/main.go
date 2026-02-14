package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	arr := make([]int, n)
	seed := uint32(42)
	for i := 0; i < n; i++ {
		seed = seed*1103515245 + 12345
		arr[i] = int((seed >> 16) & 0x7fff)
	}
	for i := 0; i < n-1; i++ {
		for j := 0; j < n-1-i; j++ {
			if arr[j] > arr[j+1] {
				arr[j], arr[j+1] = arr[j+1], arr[j]
			}
		}
	}
	fmt.Println(arr[0])
}
