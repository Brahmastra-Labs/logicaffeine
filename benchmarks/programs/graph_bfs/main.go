package main

import (
	"fmt"
	"os"
	"strconv"
)

const maxEdges = 5

func main() {
	n, _ := strconv.Atoi(os.Args[1])
	primes := [5]int{31, 37, 41, 43, 47}
	offsets := [5]int{7, 13, 17, 23, 29}
	adj := make([]int, n*maxEdges)
	adjCount := make([]int, n)
	for p := 0; p < maxEdges; p++ {
		for i := 0; i < n; i++ {
			neighbor := (i*primes[p] + offsets[p]) % n
			if neighbor != i {
				adj[i*maxEdges+adjCount[i]] = neighbor
				adjCount[i]++
			}
		}
	}
	queue := make([]int, n)
	dist := make([]int, n)
	for i := range dist { dist[i] = -1 }
	front, back := 0, 0
	queue[back] = 0; back++
	dist[0] = 0
	for front < back {
		v := queue[front]; front++
		for e := 0; e < adjCount[v]; e++ {
			u := adj[v*maxEdges+e]
			if dist[u] == -1 { dist[u] = dist[v] + 1; queue[back] = u; back++ }
		}
	}
	reachable, totalDist := 0, 0
	for i := 0; i < n; i++ {
		if dist[i] >= 0 { reachable++; totalDist += dist[i] }
	}
	fmt.Println(reachable, totalDist)
}
