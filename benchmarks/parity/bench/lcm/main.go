package main
import ("fmt";"os";"strconv")
func gcd(a,b int64) int64 { if a<0 {a=-a}; if b<0 {b=-b}; for b>0 { a,b=b,a%b }; return a }
func lcm(a,b int64) int64 { if a==0||b==0 { return 0 }; g:=gcd(a,b); p:=a*b; if p<0 {p=-p}; return p/g }
func main(){ n,_:=strconv.ParseInt(os.Args[1],10,64); var s int64
	for i:=int64(1);i<=n;i++ { s+=lcm(i,60) }; fmt.Println(s) }
