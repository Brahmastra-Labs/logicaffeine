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
		allok := len(xs) > 0; for _, b := range xs { if !(b < 128) { allok = false; break } }; if allok { checksum += 1 }
	}
	fmt.Println(checksum)
}
