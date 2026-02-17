n = ARGV[0].to_i
arr = []
seed = 42
n.times do
  seed = (seed * 1103515245 + 12345) % 2147483648
  arr << ((seed >> 16) & 0x7fff) % 1000
end
(1...n).each { |i| arr[i] = (arr[i] + arr[i-1]) % 1000000007 }
puts arr[-1]
