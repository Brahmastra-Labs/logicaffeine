package main
import ("fmt"; "os"; "strconv"; "math/bits")
func main() {
    n, _ := strconv.Atoi(os.Args[1])
    var s int64 = 0
    for i := 1; i <= n; i++ { s += int64(bits.LeadingZeros64(uint64(i))) }
    fmt.Println(s)
}
