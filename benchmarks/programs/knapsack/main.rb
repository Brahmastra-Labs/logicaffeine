n = ARGV[0].to_i
capacity = n * 5
prev = Array.new(capacity + 1, 0)
n.times do |i|
  w = (i * 17 + 3) % 50 + 1
  v = (i * 31 + 7) % 100 + 1
  curr = prev.dup
  (w..capacity).each { |j| curr[j] = prev[j - w] + v if prev[j - w] + v > curr[j] }
  prev = curr
end
puts prev[capacity]
