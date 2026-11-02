package main

import (
	"fmt"
	"os"
	"strconv"
)

func powmod(base, exp, m int64) int64 {
	acc := int64(1)
	b := base % m
	e := exp
	for e > 0 {
		if e%2 == 1 {
			acc = (acc * b) % m
		}
		e /= 2
		b = (b * b) % m
	}
	return acc
}

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	var m int64 = 1000000007
	var s int64 = 0
	for i := int64(1); i <= n; i++ {
		s += powmod(i, i, m)
	}
	fmt.Println(s)
}
