def sift_down(arr, start, stop)
  root = start
  while 2 * root + 1 <= stop
    child = 2 * root + 1
    sw = root
    sw = child if arr[sw] < arr[child]
    sw = child + 1 if child + 1 <= stop && arr[sw] < arr[child + 1]
    return if sw == root
    arr[root], arr[sw] = arr[sw], arr[root]
    root = sw
  end
end
n = ARGV[0].to_i
arr = []
seed = 42
n.times { seed = (seed * 1103515245 + 12345) % 2147483648; arr << ((seed >> 16) & 0x7fff) }
((n - 2) / 2).downto(0) { |s| sift_down(arr, s, n - 1) }
(n - 1).downto(1) { |e| arr[0], arr[e] = arr[e], arr[0]; sift_down(arr, 0, e - 1) }
checksum = 0
arr.each { |v| checksum = (checksum + v) % 1000000007 }
puts "#{arr[0]} #{arr[-1]} #{checksum}"
