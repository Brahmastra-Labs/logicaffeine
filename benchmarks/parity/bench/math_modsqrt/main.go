package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n64, _ := strconv.Atoi(os.Args[1])
	n := int64(n64)
	var s int64 = 0
	for i := int64(1); i <= n; i++ { a := new(big.Int).Mod(new(big.Int).Mul(big.NewInt(i), big.NewInt(i)), big.NewInt(1000003)); rr := new(big.Int).ModSqrt(a, big.NewInt(1000003)); rv := rr.Int64(); if 1000003-rv < rv { rv = 1000003 - rv }; s += rv }
	fmt.Println(s)
}
