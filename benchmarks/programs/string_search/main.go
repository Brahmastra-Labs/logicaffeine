package main

import (
	"fmt"
	"os"
	"strconv"
)

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	text := make([]byte, 0, n)
	pos := 0
	for pos < n {
		if pos > 0 && pos%1000 == 0 && pos+5 <= n {
			text = append(text, 'X', 'X', 'X', 'X', 'X')
			pos += 5
		} else {
			text = append(text, byte('a'+pos%5))
			pos++
		}
	}
	needle := []byte("XXXXX")
	count := 0
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
		}
	}
	fmt.Println(count)
}
