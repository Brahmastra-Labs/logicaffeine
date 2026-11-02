package main
import (
	"fmt"
	"os"
	"strconv"
	"sort"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	M := 2048
	xs := make([]int64, M)
	for k := 0; k < M; k++ { xs[k] = (int64(k) * 2654435761) % 1048576 }
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		xs[r % M] = int64(r) % 1048576
		res := make([]int64, M); copy(res, xs); sort.Slice(res, func(a, b int) bool { return res[a] < res[b] }); checksum += res[r % M]
	}
	fmt.Println(checksum)
}
