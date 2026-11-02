package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	capacity := n * 5
	weight := make([]int64, n)
	value := make([]int64, n)
	seed := int64(42)
	for i := 0; i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		weight[i] = (seed>>16)%50 + 1
		seed = (seed*1103515245 + 12345) % 2147483648
		value[i] = (seed>>16)%100 + 1
	}
	prev := make([]int64, capacity+1)
	curr := make([]int64, capacity+1)
	for i := 0; i < n; i++ {
		w := int(weight[i])
		v := value[i]
		for j := 0; j <= capacity; j++ {
			curr[j] = prev[j]
			if j >= w && prev[j-w]+v > curr[j] { curr[j] = prev[j-w] + v }
		}
		prev, curr = curr, prev
	}
	h := int64(0)
	for j := 0; j <= capacity; j++ {
		h = (h*31 + prev[j]) & 268435455
	}
	fmt.Println(prev[capacity], h)
}
