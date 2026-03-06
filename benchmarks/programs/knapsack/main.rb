n = ARGV[0].to_i
capacity = n * 5
prev = Array.new(capacity + 1, 0)
curr = Array.new(capacity + 1, 0)
n.times do |i|
  w = (i * 17 + 3) % 50 + 1
  v = (i * 31 + 7) % 100 + 1
  (0..capacity).each do |j|
    curr[j] = prev[j]
    curr[j] = prev[j - w] + v if j >= w && prev[j - w] + v > curr[j]
  end
  prev, curr = curr, prev
end
puts prev[capacity]
