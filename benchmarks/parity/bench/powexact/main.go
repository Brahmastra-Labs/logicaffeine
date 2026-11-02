package main
import ("fmt";"os";"strconv")
func powexact(base, exp int64) int64 {
	acc:=int64(1); b:=base; e:=exp
	for e>0 { if e%2==1 { acc=acc*b }; e/=2; b=b*b }
	return acc
}
func main(){
	n,_:=strconv.ParseInt(os.Args[1],10,64); var s int64
	for i:=int64(1);i<=n;i++ { s+=powexact((i%5)+1, i%13) }
	fmt.Println(s)
}
