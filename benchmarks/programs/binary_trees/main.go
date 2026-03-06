package main
import ("fmt";"os";"strconv")
func makeCheck(d int) int64 { if d==0{return 1}; return 1+makeCheck(d-1)+makeCheck(d-1) }
func main() {
	n, _ := strconv.Atoi(os.Args[1])
	mn, mx := 4, n; if mn+2 > mx { mx = mn+2 }
	fmt.Printf("stretch tree of depth %d\t check: %d\n", mx+1, makeCheck(mx+1))
	ll := makeCheck(mx)
	for d := mn; d <= mx; d += 2 {
		it := 1 << (mx - d + mn); tc := int64(0)
		for i := 0; i < it; i++ { tc += makeCheck(d) }
		fmt.Printf("%d\t trees of depth %d\t check: %d\n", it, d, tc)
	}
	fmt.Printf("long lived tree of depth %d\t check: %d\n", mx, ll)
}
