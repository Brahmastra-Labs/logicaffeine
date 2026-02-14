package main

import (
	"fmt"
	"os"
	"strconv"
)

func fib(n int64) int64 {
	if n < 2 {
		return n
	}
	return fib(n-1) + fib(n-2)
}

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	fmt.Println(fib(n))
}
