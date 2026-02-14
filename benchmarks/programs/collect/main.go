package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	m := make(map[int]int, n)
	for i := 0; i < n; i++ {
		m[i] = i * 2
	}
	found := 0
	for i := 0; i < n; i++ {
		if m[i] == i*2 {
			found++
		}
	}
	fmt.Println(found)
}
