package main
import ("bytes";"fmt";"os";"strconv";"unicode/utf8")
func main(){ n,_:=strconv.Atoi(os.Args[1])
	data:=bytes.Repeat([]byte{0xC3,0xA9}, 1000); c:=0
	for r:=0;r<n;r++ { if utf8.Valid(data) { c++ } }
	fmt.Println(c) }
