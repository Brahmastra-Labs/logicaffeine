package main
import ("fmt"; "os"; "strconv")
func main() {
	n, _ := strconv.Atoi(os.Args[1])
	coins := []int{1, 5, 10, 25, 50, 100}
	dp := make([]int64, n+1)
	dp[0] = 1
	for _, c := range coins { for j := c; j <= n; j++ { dp[j] = (dp[j] + dp[j-c]) % 1000000007 } }
	fmt.Println(dp[n])
}
