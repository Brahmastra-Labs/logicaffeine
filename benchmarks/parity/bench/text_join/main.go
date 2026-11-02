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
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		s := strings.Repeat("a", r%100) + base
		checksum += int64(len(strings.Join(strings.Split(s, "0"), "-")))
	}
	fmt.Println(checksum)
}
