package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	text := make([]byte, 0, n)
	seed := int64(42)
	for pos := 0; pos < n; pos++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		text = append(text, byte('a'+(seed>>16)%5))
	}
	needle := []byte("cabed")
	count := 0
	positions := []int64{}
	for i := 0; i <= len(text)-len(needle); i++ {
		match := true
		for j := 0; j < len(needle); j++ {
			if text[i+j] != needle[j] {
				match = false
				break
			}
		}
		if match {
			count++
			positions = append(positions, int64(i))
		}
	}
	checksum := int64(0)
	for _, p := range positions {
		checksum = (checksum*31 + p) & 268435455
	}
	fmt.Println(count, checksum)
}
