package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	var sum int64
	for i := int64(1); i <= n; i++ {
		sum = (sum + i) % 1000000007
	}
	fmt.Println(sum)
}
