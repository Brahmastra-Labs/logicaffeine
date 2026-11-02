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
	for i := int64(1); i <= n; i++ { c := i % 128; if c < 32 || c == 127 { s += 1 } }
	fmt.Println(s)
}
