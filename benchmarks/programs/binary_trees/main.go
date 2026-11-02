package main
import ("fmt";"os";"strconv")
type Node struct { left, right *Node }
func bottomUpTree(d int) *Node {
	if d > 0 { return &Node{bottomUpTree(d-1), bottomUpTree(d-1)} }
	return &Node{}
}
func check(node *Node) int64 {
	if node.left == nil { return 1 }
	return 1 + check(node.left) + check(node.right)
}
func main() {
	n, _ := strconv.Atoi(os.Args[1])
	mn, mx := 4, n; if mn+2 > mx { mx = mn+2 }
	fmt.Printf("stretch tree of depth %d\t check: %d\n", mx+1, check(bottomUpTree(mx+1)))
	ll := bottomUpTree(mx)
	for d := mn; d <= mx; d += 2 {
		it := 1 << (mx - d + mn); tc := int64(0)
		for i := 0; i < it; i++ { tc += check(bottomUpTree(d)) }
		fmt.Printf("%d\t trees of depth %d\t check: %d\n", it, d, tc)
	}
	fmt.Printf("long lived tree of depth %d\t check: %d\n", mx, check(ll))
}
