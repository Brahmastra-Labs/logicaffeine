package main

import (
	"fmt"
	"os"
	"strconv"
)

func gcd(a, b int64) int64 {
	for b > 0 { a, b = b, a%b }
	return a
}

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	var sum int64
	for i := int64(1); i <= n; i++ {
		for j := i; j <= n; j++ {
			sum += gcd(i, j)
		}
	}
	fmt.Println(sum)
}
