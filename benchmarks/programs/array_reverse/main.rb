n = ARGV[0].to_i
arr = []
seed = 42
n.times do
  seed = (seed * 1103515245 + 12345) % 2147483648
  arr << ((seed >> 16) & 0x7fff)
end
lo, hi = 0, n - 1
while lo < hi
  arr[lo], arr[hi] = arr[hi], arr[lo]
  lo += 1; hi -= 1
end
puts "#{arr[0]} #{arr[-1]} #{arr[n / 2]}"
