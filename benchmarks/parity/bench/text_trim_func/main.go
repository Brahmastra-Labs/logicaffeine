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
		b := []byte(s); i := 0; for i < len(b) && b[i] == 97 { i++ }; j := len(b); for j > i && b[j-1] == 97 { j-- }; checksum += int64(j - i)
	}
	fmt.Println(checksum)
}
