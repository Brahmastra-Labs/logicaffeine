def qs(arr, lo, hi)
  return if lo >= hi
  pivot = arr[hi]; i = lo
  (lo...hi).each do |j|
    if arr[j] <= pivot; arr[i], arr[j] = arr[j], arr[i]; i += 1; end
  end
  arr[i], arr[hi] = arr[hi], arr[i]
  qs(arr, lo, i - 1)
  qs(arr, i + 1, hi)
end

n = ARGV[0].to_i
arr = []
seed = 42
n.times { seed = (seed * 1103515245 + 12345) % 2147483648; arr << ((seed >> 16) & 0x7fff) }
qs(arr, 0, n - 1)
checksum = 0
arr.each { |v| checksum = (checksum + v) % 1000000007 }
puts "#{arr[0]} #{arr[-1]} #{checksum}"
