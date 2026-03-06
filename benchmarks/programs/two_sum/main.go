package main
import ("fmt";"os";"strconv")
func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	arr := make([]int64, n)
	seed := int64(42)
	for i := int64(0); i < n; i++ { seed = (seed*1103515245+12345)%2147483648; arr[i] = ((seed>>16)&0x7fff)%n }
	seen := make(map[int64]bool)
	count := int64(0)
	for _, x := range arr {
		c := n - x
		if c >= 0 && seen[c] { count++ }
		seen[x] = true
	}
	fmt.Println(count)
}
