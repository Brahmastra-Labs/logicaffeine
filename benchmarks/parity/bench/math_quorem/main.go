package main
import("fmt";"os";"strconv";"math")
func main(){n64,_:=strconv.Atoi(os.Args[1]);n:=int64(n64);var s int64=0;_=math.Sqrt
for i:=int64(1);i<=n;i++{s+=(i/7)+(i%7)}
fmt.Println(s)}
