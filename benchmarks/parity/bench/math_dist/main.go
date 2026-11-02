package main
import("fmt";"os";"strconv";"math")
func main(){n64,_:=strconv.Atoi(os.Args[1]);n:=int64(n64);var s int64=0;_=math.Sqrt
for i:=int64(1);i<=n;i++{dx:=float64(i)/1000.0-3.0;dy:=float64(i)/700.0-4.0;s+=int64(math.Sqrt(dx*dx+dy*dy)*1000000.0)}
fmt.Println(s)}
