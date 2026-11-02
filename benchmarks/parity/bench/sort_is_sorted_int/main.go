package main
import ("fmt";"os";"sort";"strconv")
func main(){ n,_:=strconv.Atoi(os.Args[1]); m:=1000
    xs:=make([]int,m); for i:=0;i<m;i++ { xs[i]=i }
    s:=0
    for r:=0;r<n;r++ { if sort.IntsAreSorted(xs) { s++ } }
    fmt.Println(s) }
