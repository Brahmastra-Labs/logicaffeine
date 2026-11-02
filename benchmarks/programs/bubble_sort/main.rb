n = ARGV[0].to_i
arr = Array.new(n)
seed = 42
n.times do |i|
  seed = (seed * 1103515245 + 12345) & 0xffffffff
  arr[i] = (seed >> 16) & 0x7fff
end
(n - 1).times do |i|
  (n - 1 - i).times do |j|
    if arr[j] > arr[j + 1]
      arr[j], arr[j + 1] = arr[j + 1], arr[j]
    end
  end
end
checksum = 0
arr.each { |v| checksum = (checksum * 31 + v) & 268435455 }
puts "#{arr[0]} #{arr[-1]} #{checksum}"
