package main

import (
	"fmt"
	"math/big"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	bigdiv, _ := new(big.Int).SetString("100000000000000000000000000000000000000000000000000007", 10)
	d, _ := new(big.Int).SetString("100000000000000000003", 10)
	mod, _ := new(big.Int).SetString("10000000000000000000000000000000000000009", 10)
	acc := big.NewInt(0)
	t := new(big.Int)
	q := new(big.Int)
	bi := new(big.Int)
	for i := int64(1); i <= n; i++ {
		bi.SetInt64(i)
		t.Add(bigdiv, bi)
		q.Quo(t, d)
		acc.Add(acc, q)
		acc.Mod(acc, mod)
	}
	fmt.Println(acc.String())
}
