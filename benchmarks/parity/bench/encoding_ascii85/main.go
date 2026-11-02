package main
import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"encoding/ascii85"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	base := strings.Repeat("abcABC012 ", 400)
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		s := strings.Repeat("a", r%100) + base
		buf := make([]byte, ascii85.MaxEncodedLen(len(s))); n := ascii85.Encode(buf, []byte(s)); checksum += int64(n)
	}
	fmt.Println(checksum)
}
