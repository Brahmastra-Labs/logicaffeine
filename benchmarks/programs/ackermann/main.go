package main

import (
	"fmt"
	"os"
	"strconv"
)

func ackermann(m, n int64) int64 {
	if m == 0 {
		return n + 1
	}
	if n == 0 {
		return ackermann(m-1, 1)
	}
	return ackermann(m-1, ackermann(m, n-1))
}

func main() {
	m, _ := strconv.ParseInt(os.Args[1], 10, 64)
	fmt.Println(ackermann(3, m))
}
