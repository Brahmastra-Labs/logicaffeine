package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	arr := make([]int64, n)
	for i := 0; i < n; i++ {
		arr[i] = (int64(i)*7 + 3) % 1000000
	}
	var h int64
	for _, v := range arr {
		h = (h*31 + v) & 268435455
	}
	fmt.Println(h)
}
