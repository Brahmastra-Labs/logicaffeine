package main
import ("fmt";"os";"strconv")
func modinverse(x, m int64) int64 {
	oldR:=x%m; if oldR<0 { oldR+=m }
	r:=m; oldS:=int64(1); s:=int64(0)
	for r>0 {
		q:=oldR/r
		tR:=oldR-q*r; oldR=r; r=tR
		tS:=oldS-q*s; oldS=s; s=tS
	}
	if oldR>1 { return 0 }
	inv:=oldS%m
	if inv<0 { return inv+m }
	return inv
}
func main(){
	n,_:=strconv.ParseInt(os.Args[1],10,64); var m int64=1000000007; var sum int64
	for i:=int64(1);i<=n;i++ { sum+=modinverse(i,m) }
	fmt.Println(sum)
}
