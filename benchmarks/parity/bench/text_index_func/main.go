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
		idx := -1; for i := 0; i < len(s); i++ { if s[i] == 48 { idx = i; break } }; if idx >= 0 { checksum += int64(idx) + 1 }
	}
	fmt.Println(checksum)
}
