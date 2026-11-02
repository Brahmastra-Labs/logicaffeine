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
	for i := int64(1); i <= n; i++ { s += int64(len(strconv.AppendQuoteRune([]byte("x"), rune(65 + i % 26)))) }
	fmt.Println(s)
}
