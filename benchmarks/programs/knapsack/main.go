package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	capacity := n * 5
	prev := make([]int64, capacity+1)
	curr := make([]int64, capacity+1)
	for i := 0; i < n; i++ {
		w := (i*17 + 3) % 50 + 1
		v := int64((i*31 + 7) % 100 + 1)
		for j := 0; j <= capacity; j++ {
			curr[j] = prev[j]
			if j >= w && prev[j-w]+v > curr[j] { curr[j] = prev[j-w] + v }
		}
		prev, curr = curr, prev
	}
	fmt.Println(prev[capacity])
}
