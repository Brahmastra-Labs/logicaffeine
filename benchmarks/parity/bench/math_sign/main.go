package main
import ("fmt";"os";"strconv")
func main(){ n,_:=strconv.Atoi(os.Args[1]); var sum int64=0
  for i:=int64(1);i<=int64(n);i++{ x:=i-10000000; if x>0 {sum++} else if x<0 {sum--} }
  fmt.Println(sum) }
