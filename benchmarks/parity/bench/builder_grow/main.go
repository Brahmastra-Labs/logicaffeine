package main
import (
	"fmt"
	"os"
	"strconv"
	"bytes"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	_ = fmt.Sprint
	var sum int64 = 0
	for i := 0; i < reps; i++ { var b bytes.Buffer; b.Grow(100); sum += int64(b.Len()) }
	fmt.Println(sum)
}
