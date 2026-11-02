package main

import (
	"fmt"
	"math/big"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	base, _ := new(big.Int).SetString("1000000000000000000000000000007", 10)
	mod, _ := new(big.Int).SetString("10000000000000000000000000000000000000009", 10)
	acc := big.NewInt(0)
	t := new(big.Int)
	bi := new(big.Int)
	term := new(big.Int)
	for i := int64(1); i <= n; i++ {
		bi.SetInt64(i)
		term.Add(base, bi)
		t.Add(acc, term)
		acc.Mod(t, mod)
	}
	fmt.Println(acc.String())
}
