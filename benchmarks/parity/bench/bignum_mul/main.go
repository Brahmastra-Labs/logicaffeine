package main

import (
	"fmt"
	"math/big"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	k, _ := new(big.Int).SetString("10000000000000000000000003", 10)
	mod, _ := new(big.Int).SetString("10000000000000000000000000000000000000009", 10)
	acc := big.NewInt(1)
	t := new(big.Int)
	bi := new(big.Int)
	for i := int64(1); i <= n; i++ {
		t.Mul(acc, k)
		bi.SetInt64(i)
		t.Add(t, bi)
		acc.Mod(t, mod)
	}
	fmt.Println(acc.String())
}
