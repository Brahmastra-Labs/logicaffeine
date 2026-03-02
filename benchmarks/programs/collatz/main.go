package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	var total int64
	for i := int64(1); i <= n; i++ {
		k := i
		for k != 1 {
			if k%2 == 0 {
				k /= 2
			} else {
				k = 3*k + 1
			}
			total++
		}
	}
	fmt.Println(total)
}
