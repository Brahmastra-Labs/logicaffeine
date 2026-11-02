package main
import ("fmt";"os";"strconv")
func isqrt(n int64) int64 { if n<2 { return n }; x:=n; y:=(x+1)/2; for y<x { x=y; y=(x+n/x)/2 }; return x }
func main(){ n,_:=strconv.ParseInt(os.Args[1],10,64); var s int64
	for i:=int64(1);i<=n;i++ { s+=isqrt(i) }; fmt.Println(s) }
