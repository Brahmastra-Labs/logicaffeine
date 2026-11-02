package main
import ("fmt";"os";"strconv")
func fact(n int64) int64 { r:=int64(1); for i:=int64(2);i<=n;i++ { r*=i }; return r }
func main(){ n,_:=strconv.ParseInt(os.Args[1],10,64); var s int64
	for i:=int64(1);i<=n;i++ { s+=fact((i%12)+1) }; fmt.Println(s) }
