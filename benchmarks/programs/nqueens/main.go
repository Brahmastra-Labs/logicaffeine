package main

import (
	"fmt"
	"os"
	"strconv"
)

func solve(row, cols, diag1, diag2, n int) int {
	if row == n {
		return 1
	}
	count := 0
	available := ((1 << n) - 1) &^ (cols | diag1 | diag2)
	for available != 0 {
		bit := available & (-available)
		available ^= bit
		count += solve(row+1, cols|bit, (diag1|bit)<<1, (diag2|bit)>>1, n)
	}
	return count
}

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	fmt.Println(solve(0, 0, 0, 0, n))
}
