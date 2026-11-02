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
		mn := xs[0]; for _, v := range xs { if v < mn { mn = v } }; checksum += mn
	}
	fmt.Println(checksum)
}
