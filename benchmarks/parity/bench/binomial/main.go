package main

import (
	"fmt"
	"os"
	"strconv"
)

func binomial(n, k int64) int64 {
	if k < 0 {
		return 0
	}
	if k > n {
		return 0
	}
	kk := k
	if kk > n-kk {
		kk = n - kk
	}
	acc := int64(1)
	i := int64(1)
	for i <= kk {
		acc = acc * (n - kk + i) / i
		i++
	}
	return acc
}

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	var s int64 = 0
	for i := int64(1); i <= n; i++ {
		s += binomial(30, (s+i)%16)
	}
	fmt.Println(s)
}
