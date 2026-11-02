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
	for i := 0; i < reps; i++ { var b bytes.Buffer; b.WriteString("abcdefgh"); sum += int64(len(b.Bytes())) }
	fmt.Println(sum)
}
