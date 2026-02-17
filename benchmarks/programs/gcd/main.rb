def gcd(a, b)
  while b > 0; a, b = b, a % b; end
  a
end
n = ARGV[0].to_i
sum = 0
(1..n).each { |i| (i..n).each { |j| sum += gcd(i, j) } }
puts sum
