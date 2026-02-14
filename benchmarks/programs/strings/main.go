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
	for i := 0; i < n; i++ {
		b.WriteString(strconv.Itoa(i))
		b.WriteByte(' ')
	}
	result := b.String()
	spaces := 0
	for _, c := range result {
		if c == ' ' {
			spaces++
		}
	}
	fmt.Println(spaces)
}
