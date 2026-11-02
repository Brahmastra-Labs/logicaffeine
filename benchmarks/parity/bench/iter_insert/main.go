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
		pos := int(r) % M; res := make([]int64, 0, M+1); res = append(res, xs[:pos]...); res = append(res, int64(r)); res = append(res, xs[pos:]...); checksum += res[pos]
	}
	fmt.Println(checksum)
}
