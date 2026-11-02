package main
import (
	"fmt"
	"os"
	"strconv"
	"strings"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	base := strings.Repeat("abcABC012 ", 400)
	sub := "XYZ"
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		s := strings.Repeat("a", r%100) + base
		if i := strings.IndexAny(s, sub); i >= 0 { checksum += int64(i) + 1 }
	}
	fmt.Println(checksum)
}
