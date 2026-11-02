package main

import (
	"fmt"
	"os"
	"strconv"
	"math"
)

func main() {
	n64, _ := strconv.Atoi(os.Args[1])
	n := int64(n64)
	var s int64 = 0
	for i := int64(1); i <= n; i++ { _, e := math.Frexp(float64(i)/1000.0); s += int64(e) }
	fmt.Println(s)
}
