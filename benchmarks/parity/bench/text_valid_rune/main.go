package main
import ("fmt";"os";"strconv";"unicode/utf8")
func main(){ n,_:=strconv.Atoi(os.Args[1]); c:=0
	for r:=0;r<n;r++ { if utf8.ValidRune(rune(r)) { c++ } }
	fmt.Println(c) }
