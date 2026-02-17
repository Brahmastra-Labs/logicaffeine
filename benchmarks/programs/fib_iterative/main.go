package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	var a, b int64
	b = 1
	for i := int64(0); i < n; i++ {
		temp := b
		b = (a + b) % 1000000007
		a = temp
	}
	fmt.Println(a)
}
