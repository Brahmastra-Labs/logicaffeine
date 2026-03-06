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
	for i := 0; i < n; i++ {
		for j := 0; j < n; j++ {
			a[i*n+j] = int64((i*n + j) % 100)
			b[i*n+j] = int64((j*n + i) % 100)
		}
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
		checksum = (checksum + v) % MOD
	}
	fmt.Println(checksum)
}
