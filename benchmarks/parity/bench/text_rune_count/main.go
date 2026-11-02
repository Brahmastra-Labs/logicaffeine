package main
import ("fmt";"os";"strconv";"strings";"unicode/utf8")
func main(){ n,_:=strconv.Atoi(os.Args[1]); const MOD=1000000007
	base:=strings.Repeat("héllo wörld ", 200); s:=0
	for r:=0;r<n;r++ { s=(s+utf8.RuneCountInString(base))%MOD }
	fmt.Println(s) }
