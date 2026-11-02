package main

import (
	"fmt"
	"os"
	"strconv"
	"strings"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	var b strings.Builder
	b.Grow(n * 6)
	var buf [20]byte
	for i := 0; i < n; i++ {
		b.Write(strconv.AppendInt(buf[:0], int64(i), 10))
		b.WriteByte(' ')
	}
	result := b.String()
	spaces := 0
	for i := 0; i < len(result); i++ {
		if result[i] == ' ' {
			spaces++
		}
	}
	h := int64(0)
	for i := 0; i < len(result); i++ {
		h = (h*31 + int64(result[i])) & 268435455
	}
	fmt.Println(spaces, h)
}
