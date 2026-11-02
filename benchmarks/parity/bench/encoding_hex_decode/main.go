package main
import (
	"fmt"
	"os"
	"strconv"
	"strings"
	"encoding/hex"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	base := strings.Repeat("abcABC012 ", 400)
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		s := strings.Repeat("a", r%100) + base
		d, _ := hex.DecodeString(hex.EncodeToString([]byte(s))); checksum += int64(len(d))
	}
	fmt.Println(checksum)
}
