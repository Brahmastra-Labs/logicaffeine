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
	for i := int64(1); i <= n; i++ { if math.Signbit(float64((i%2)*2-1)) { s += 1 } }
	fmt.Println(s)
}
