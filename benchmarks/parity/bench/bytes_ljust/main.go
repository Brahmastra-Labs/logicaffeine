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
		res := make([]byte, M+100); copy(res, xs); for i := M; i < M+100; i++ { res[i] = 32 }; checksum += int64(res[r % M])
	}
	fmt.Println(checksum)
}
