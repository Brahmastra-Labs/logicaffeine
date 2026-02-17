package main
import ("fmt"; "os"; "strconv")
func main() {
	n, _ := strconv.ParseInt(os.Args[1], 10, 64)
	sum, sign := 0.0, 1.0
	for k := int64(0); k < n; k++ { sum += sign / (2.0*float64(k) + 1.0); sign = -sign }
	fmt.Printf("%.15f\n", sum*4.0)
}
