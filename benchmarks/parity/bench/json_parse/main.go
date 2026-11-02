package main
import (
	"fmt"
	"os"
	"strconv"
	"encoding/json"
)
func main() {
	reps, _ := strconv.Atoi(os.Args[1])
	tail := ""
	for k := 1; k <= 500; k++ { tail += "," + strconv.Itoa(k) }
	tail += "]"
	var checksum int64 = 0
	for r := 0; r < reps; r++ {
		js := "[" + strconv.Itoa(r % 1000) + tail
		var v interface{}; json.Unmarshal([]byte(js), &v); out, _ := json.Marshal(v); checksum += int64(len(out))
	}
	fmt.Println(checksum)
}
