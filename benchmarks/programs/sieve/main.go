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
	h := 0
	for i := 2; i <= limit; i++ {
		v := 0
		if !sieve[i] {
			v = 1
			count++
			for j := i * i; j <= limit; j += i {
				sieve[j] = true
			}
		}
		h = (h*31 + v) & 268435455
	}
	fmt.Println(count, h)
}
