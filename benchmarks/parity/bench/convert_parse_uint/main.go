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
	for i := int64(1); i <= n; i++ { n, _ := strconv.ParseUint(strconv.Itoa(int(i)), 10, 64); s += int64(n) }
	fmt.Println(s)
}
