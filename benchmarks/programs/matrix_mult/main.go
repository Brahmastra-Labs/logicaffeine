package main

import (
	"fmt"
	"os"
	"strconv"
)

const MOD = 1000000007

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	a := make([]int64, n*n)
	b := make([]int64, n*n)
	c := make([]int64, n*n)
	seed := int64(42)
	for i := 0; i < n*n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		a[i] = (seed >> 16) % 100
	}
	for i := 0; i < n*n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		b[i] = (seed >> 16) % 100
	}
	for i := 0; i < n; i++ {
		for k := 0; k < n; k++ {
			for j := 0; j < n; j++ {
				c[i*n+j] = (c[i*n+j] + a[i*n+k]*b[k*n+j]) % MOD
			}
		}
	}
	checksum := int64(0)
	for _, v := range c {
		checksum = (checksum*31 + v) & 268435455
	}
	fmt.Println(checksum)
}
