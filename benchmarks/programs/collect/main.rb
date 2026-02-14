n = ARGV[0].to_i
map = {}
n.times { |i| map[i] = i * 2 }
found = 0
n.times { |i| found += 1 if map[i] == i * 2 }
puts found
