def merge_sort(arr)
  return arr if arr.length < 2
  mid = arr.length / 2
  left = merge_sort(arr[0...mid])
  right = merge_sort(arr[mid..])
  result = []
  i = j = 0
  while i < left.length && j < right.length
    if left[i] <= right[j]; result << left[i]; i += 1
    else result << right[j]; j += 1; end
  end
  result.concat(left[i..]).concat(right[j..])
end

n = ARGV[0].to_i
arr = []
seed = 42
n.times { seed = (seed * 1103515245 + 12345) % 2147483648; arr << ((seed >> 16) & 0x7fff) }
arr = merge_sort(arr)
checksum = 0
arr.each { |v| checksum = (checksum + v) % 1000000007 }
puts "#{arr[0]} #{arr[-1]} #{checksum}"
