package main
import ("fmt";"os";"strconv")
func main(){ n,_:=strconv.Atoi(os.Args[1]); var sum int64=0
  for i:=int64(1);i<=int64(n);i++{ v:=i; if v<5000000 {v=5000000}; if v>15000000 {v=15000000}; sum+=v }
  fmt.Println(sum) }
