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
	var sum int64 = 0
	for r := 0; r < reps; r++ {
		sum += bytes.NewReader(xs).Size()
	}
	fmt.Println(sum)
}
