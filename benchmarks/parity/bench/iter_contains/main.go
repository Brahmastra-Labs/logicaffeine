package main
import (
	"fmt"
	"os"
	"strconv"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	M := 4096
	xs := make([]int64, M)
	for k := 0; k < M; k++ { xs[k] = (int64(k) * 2654435761) % 1048576 }
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		xs[r % M] = int64(r) % 1048576
		t := int64(r % 2097152); for _, v := range xs { if v == t { checksum += 1; break } }
	}
	fmt.Println(checksum)
}
