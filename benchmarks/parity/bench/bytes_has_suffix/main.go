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
	sub := []byte{72, 101, 108}
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		xs[r % M] = byte(r % 256)
		if bytes.HasSuffix(xs, sub) { checksum += 1 }
	}
	fmt.Println(checksum)
}
