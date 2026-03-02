n = ARGV[0].to_i
counts = Array.new(1000, 0)
seed = 42
n.times do
  seed = (seed * 1103515245 + 12345) % 2147483648
  counts[((seed >> 16) & 0x7fff) % 1000] += 1
end
max_freq = max_idx = distinct = 0
1000.times do |i|
  distinct += 1 if counts[i] > 0
  if counts[i] > max_freq; max_freq = counts[i]; max_idx = i; end
end
puts "#{max_freq} #{max_idx} #{distinct}"
