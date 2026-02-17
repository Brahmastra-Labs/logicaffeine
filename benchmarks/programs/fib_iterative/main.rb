n = ARGV[0].to_i
a, b = 0, 1
n.times { a, b = b, (a + b) % 1000000007 }
puts a
