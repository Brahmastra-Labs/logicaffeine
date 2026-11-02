def merge_sort(arr)
  n = arr.length
  return if n < 2
  mid = n / 2
  left = arr[0...mid]
  right = arr[mid..]
  merge_sort(left)
  merge_sort(right)
  i = j = k = 0
  while i < mid && j < n - mid
    if left[i] <= right[j]; arr[k] = left[i]; i += 1
    else arr[k] = right[j]; j += 1; end
    k += 1
  end
  while i < mid; arr[k] = left[i]; i += 1; k += 1; end
  while j < n - mid; arr[k] = right[j]; j += 1; k += 1; end
end

n = ARGV[0].to_i
arr = []
seed = 42
n.times { seed = (seed * 1103515245 + 12345) % 2147483648; arr << ((seed >> 16) & 0x7fff) }
merge_sort(arr)
checksum = 0
arr.each { |v| checksum = (checksum * 31 + v) & 268435455 }
puts "#{arr[0]} #{arr[-1]} #{checksum}"
