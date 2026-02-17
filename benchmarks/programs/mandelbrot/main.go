package main
import ("fmt"; "os"; "strconv")
func main() {
	n, _ := strconv.Atoi(os.Args[1])
	count := 0
	for y := 0; y < n; y++ {
		for x := 0; x < n; x++ {
			cr := 2.0*float64(x)/float64(n) - 1.5
			ci := 2.0*float64(y)/float64(n) - 1.0
			zr, zi := 0.0, 0.0
			inside := true
			for i := 0; i < 50; i++ {
				t := zr*zr - zi*zi + cr; zi = 2*zr*zi + ci; zr = t
				if zr*zr+zi*zi > 4 { inside = false; break }
			}
			if inside { count++ }
		}
	}
	fmt.Println(count)
}
