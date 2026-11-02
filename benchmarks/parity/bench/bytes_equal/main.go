package main
import (
	"fmt"
	"os"
	"strconv"
	"bytes"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	M := 4096
	xs := make([]byte, M)
	for k := 0; k < M; k++ { xs[k] = byte((k * 2654435761) % 256) }
	ys := make([]byte, M)
	for k := 0; k < M; k++ { ys[k] = byte((k*1013904223 + 7) % 256) }
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		xs[r % M] = byte(r % 256)
		if bytes.Equal(xs, ys) { checksum += 1 }
	}
	fmt.Println(checksum)
}
