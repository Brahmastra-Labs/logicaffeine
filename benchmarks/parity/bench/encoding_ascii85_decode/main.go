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
		eb := make([]byte, ascii85.MaxEncodedLen(len(s))); en := ascii85.Encode(eb, []byte(s)); db := make([]byte, len(s)+16); dn, _, _ := ascii85.Decode(db, eb[:en], true); checksum += int64(dn)
	}
	fmt.Println(checksum)
}
