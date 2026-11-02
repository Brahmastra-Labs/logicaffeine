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
		res := make([]byte, len(s)); for i := 0; i < len(s); i++ { res[i] = byte((int(s[i]) + 1) % 128) }; checksum += int64(len(res))
	}
	fmt.Println(checksum)
}
