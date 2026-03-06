package main
import ("fmt";"math";"os";"strconv")
func a(i,j int) float64 { return 1.0/float64((i+j)*(i+j+1)/2+i+1) }
func mulAv(n int, v, r []float64) { for i:=0;i<n;i++{r[i]=0;for j:=0;j<n;j++{r[i]+=a(i,j)*v[j]}} }
func mulAtv(n int, v, r []float64) { for i:=0;i<n;i++{r[i]=0;for j:=0;j<n;j++{r[i]+=a(j,i)*v[j]}} }
func mulAtAv(n int, v, r, t []float64) { mulAv(n,v,t); mulAtv(n,t,r) }
func main() {
	n, _ := strconv.Atoi(os.Args[1])
	u := make([]float64, n); v := make([]float64, n); t := make([]float64, n)
	for i := range u { u[i] = 1 }
	for i := 0; i < 10; i++ { mulAtAv(n,u,v,t); mulAtAv(n,v,u,t) }
	vBv, vv := 0.0, 0.0
	for i := 0; i < n; i++ { vBv += u[i]*v[i]; vv += v[i]*v[i] }
	fmt.Printf("%.9f\n", math.Sqrt(vBv/vv))
}
