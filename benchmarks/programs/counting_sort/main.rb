n = ARGV[0].to_i
arr = []
seed = 42
n.times do
  seed = (seed * 1103515245 + 12345) % 2147483648
  arr << (seed >> 16) % 1000
end
counts = Array.new(1000, 0)
arr.each { |v| counts[v] += 1 }
sorted = []
1000.times { |v| counts[v].times { sorted << v } }
checksum = 0
sorted.each { |v| checksum = (checksum + v) % 1000000007 }
puts "#{sorted[0]} #{sorted[-1]} #{checksum}"
