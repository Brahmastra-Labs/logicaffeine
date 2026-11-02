n = ARGV[0].to_i
result = String.new(capacity: n * 6)
n.times { |i| result << i.to_s << " " }
spaces = 0
result.each_byte { |b| spaces += 1 if b == 32 }
h = 0
result.each_byte { |b| h = (h * 31 + b) & 268435455 }
puts "#{spaces} #{h}"
