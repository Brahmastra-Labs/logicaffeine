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
		arr[i] = ((seed >> 16) & 0x7fff) % 1000
	}
	for i := 1; i < n; i++ {
		arr[i] = (arr[i] + arr[i-1]) % 1000000007
	}
	fmt.Println(arr[n-1])
}
