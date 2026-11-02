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
	var b bytes.Buffer
	for i := 0; i < reps; i++ { b.WriteString("abcdefgh") }
	fmt.Println(int64(b.Len()))
}
