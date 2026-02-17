n = ARGV[0].to_i
arr = (0...n).map { |i| (i * 7 + 3) % 1000000 }
sum = 0
arr.each { |v| sum = (sum + v) % 1000000007 }
puts sum
