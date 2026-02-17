n = ARGV[0].to_i
total = 0
(1..n).each do |i|
  k = i
  while k != 1
    if k % 2 == 0 then k /= 2 else k = 3 * k + 1 end
    total += 1
  end
end
puts total
