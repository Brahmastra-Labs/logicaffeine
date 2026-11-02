package main
import ("fmt";"os";"strconv";"unicode/utf8")
func main(){ n,_:=strconv.Atoi(os.Args[1]); s:=0
	for i:=0;i<n;i++ {
		cp:=i%1112064
		if cp>=55296 { cp+=2048 }
		s+=utf8.RuneLen(rune(cp))
	}
	fmt.Println(s) }
