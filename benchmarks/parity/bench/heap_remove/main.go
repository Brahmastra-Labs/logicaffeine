package main
import (
	"fmt"
	"os"
	"strconv"
	"container/heap"
)
type IH []int64
func (h IH) Len() int { return len(h) }
func (h IH) Less(i, j int) bool { return h[i] < h[j] }
func (h IH) Swap(i, j int) { h[i], h[j] = h[j], h[i] }
func (h *IH) Push(x any) { *h = append(*h, x.(int64)) }
func (h *IH) Pop() any { o := *h; n := len(o); x := o[n-1]; *h = o[:n-1]; return x }
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	M := 4096
	xs := make([]int64, M)
	for k := 0; k < M; k++ { xs[k] = (int64(k) * 2654435761) % 1048576 }
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		xs[r % M] = int64(r) % 1048576
		h := IH(append([]int64{}, xs...)); heap.Init(&h); heap.Remove(&h, r % M); checksum += int64(h.Len())
	}
	fmt.Println(checksum)
}
