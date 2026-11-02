package main
import (
	"fmt"
	"os"
	"strconv"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	M := 4096
	xs := make([]byte, M)
	for k := 0; k < M; k++ { xs[k] = byte((k * 2654435761) % 256) }
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		xs[r % M] = byte(r % 256)
		cased := false; prevCased := false; ok := true; for _, b := range xs { up := b >= 65 && b <= 90; lo := b >= 97 && b <= 122; if up { if prevCased { ok = false; break }; cased = true; prevCased = true } else if lo { if !prevCased { ok = false; break }; cased = true; prevCased = true } else { prevCased = false } }; if ok && cased { checksum += 1 }
	}
	fmt.Println(checksum)
}
