MAX_EDGES = 5
n = ARGV[0].to_i
primes = [31, 37, 41, 43, 47]
offsets = [7, 13, 17, 23, 29]
adj = Array.new(n * MAX_EDGES, 0)
adj_count = Array.new(n, 0)
MAX_EDGES.times{|p|
  n.times{|i|
    neighbor = (i * primes[p] + offsets[p]) % n
    if neighbor != i
      adj[i * MAX_EDGES + adj_count[i]] = neighbor
      adj_count[i] += 1
    end
  }
}
queue = Array.new(n, 0)
dist = Array.new(n, -1)
front = 0; back = 0
queue[back] = 0; back += 1
dist[0] = 0
while front < back
  v = queue[front]; front += 1
  adj_count[v].times{|e|
    u = adj[v * MAX_EDGES + e]
    if dist[u] == -1
      dist[u] = dist[v] + 1
      queue[back] = u; back += 1
    end
  }
end
reachable = 0; total_dist = 0
n.times{|i|
  if dist[i] >= 0; reachable += 1; total_dist += dist[i]; end
}
puts "#{reachable} #{total_dist}"
