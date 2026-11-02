n = ARGV[0].to_i
capacity = n * 5
weight = Array.new(n, 0)
value = Array.new(n, 0)
seed = 42
n.times do |i|
  seed = (seed * 1103515245 + 12345) % 2147483648
  weight[i] = (seed >> 16) % 50 + 1
  seed = (seed * 1103515245 + 12345) % 2147483648
  value[i] = (seed >> 16) % 100 + 1
end
prev = Array.new(capacity + 1, 0)
curr = Array.new(capacity + 1, 0)
n.times do |i|
  w = weight[i]
  v = value[i]
  (0..capacity).each do |j|
    curr[j] = prev[j]
    curr[j] = prev[j - w] + v if j >= w && prev[j - w] + v > curr[j]
  end
  prev, curr = curr, prev
end
h = 0
(0..capacity).each do |j|
  h = (h * 31 + prev[j]) & 268435455
end
puts "#{prev[capacity]} #{h}"
