package main
import ("fmt"; "os"; "strconv")
func main() {
	n, _ := strconv.Atoi(os.Args[1])
	perm1 := make([]int, n)
	for i := range perm1 { perm1[i] = i }
	count := make([]int, n)
	perm := make([]int, n)
	maxFlips, checksum, permCount, r := 0, 0, 0, n
	for {
		for r > 1 { count[r-1] = r; r-- }
		copy(perm, perm1)
		flips := 0
		for perm[0] != 0 {
			k := perm[0] + 1
			for i, j := 0, k-1; i < j; i, j = i+1, j-1 { perm[i], perm[j] = perm[j], perm[i] }
			flips++
		}
		if flips > maxFlips { maxFlips = flips }
		if permCount%2 == 0 { checksum += flips } else { checksum -= flips }
		permCount++
		for {
			if r == n { fmt.Printf("%d\n%d\n", checksum, maxFlips); return }
			p0 := perm1[0]
			for i := 0; i < r; i++ { perm1[i] = perm1[i+1] }
			perm1[r] = p0
			count[r]--
			if count[r] > 0 { break }
			r++
		}
	}
}
