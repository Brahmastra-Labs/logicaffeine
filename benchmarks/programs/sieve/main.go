package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	limit, _ := strconv.Atoi(os.Args[1])
	sieve := make([]bool, limit+1)
	count := 0
	for i := 2; i <= limit; i++ {
		if !sieve[i] {
			count++
			for j := i * i; j <= limit; j += i {
				sieve[j] = true
			}
		}
	}
	fmt.Println(count)
}
