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
	checksum := 0
	for _, v := range arr {
		checksum = (checksum*31 + v) & 268435455
	}
	fmt.Printf("%d %d %d\n", arr[0], arr[n-1], checksum)
}
