package main
import (
	"fmt"
	"os"
	"strconv"
	"sort"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	M := 4096
	xs := make([]int64, M)
	for k := 0; k < M; k++ { xs[k] = (int64(k) * 2654435761) % 1048576 }
	sort.Slice(xs, func(a, b int) bool { return xs[a] < xs[b] })
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		t := int64(r % 1048576); i := sort.Search(M, func(j int) bool { return xs[j] >= t }); if i < M && xs[i] == t { checksum += int64(i) + 1 }
	}
	fmt.Println(checksum)
}
