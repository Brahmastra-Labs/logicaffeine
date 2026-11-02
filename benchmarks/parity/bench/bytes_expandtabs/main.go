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
		col := 0; n := 0; for _, b := range xs { if b == 9 { sp := 8 - (col % 8); n += sp; col += sp } else if b == 10 || b == 13 { n++; col = 0 } else { n++; col++ } }; checksum += int64(n)
	}
	fmt.Println(checksum)
}
