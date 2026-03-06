package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	var count int64
	for i := int64(2); i <= n; i++ {
		isPrime := true
		for d := int64(2); d*d <= i; d++ {
			if i%d == 0 { isPrime = false; break }
		}
		if isPrime { count++ }
	}
	fmt.Println(count)
}
