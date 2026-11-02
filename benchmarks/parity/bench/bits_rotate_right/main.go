package main
import("fmt";"os";"strconv")
func main(){n64,_:=strconv.Atoi(os.Args[1]);n:=int64(n64);var s int64=0
for i:=int64(1);i<=n;i++{x:=uint32(i);s+=int64((x>>7)|(x<<25))}
fmt.Println(s)}
