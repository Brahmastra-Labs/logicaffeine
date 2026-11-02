n = ARGV[0].to_i
arr = (0...n).map { |i| (i * 7 + 3) % 1000000 }
h = 0
arr.each { |v| h = (h * 31 + v) & 268435455 }
puts h
