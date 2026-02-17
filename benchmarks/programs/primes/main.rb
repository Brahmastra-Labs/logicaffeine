n = ARGV[0].to_i
count = 0
(2..n).each do |i|
  is_prime = true
  d = 2
  while d * d <= i
    if i % d == 0; is_prime = false; break; end
    d += 1
  end
  count += 1 if is_prime
end
puts count
