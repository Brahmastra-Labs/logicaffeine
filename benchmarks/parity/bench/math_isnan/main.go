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
	for i := int64(1); i <= n; i++ { if math.IsNaN(float64(i)) { s += 1 } }
	fmt.Println(s)
}
