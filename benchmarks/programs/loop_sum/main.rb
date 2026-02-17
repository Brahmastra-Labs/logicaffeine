n = ARGV[0].to_i
sum = 0
(1..n).each { |i| sum = (sum + i) % 1000000007 }
puts sum
