limit = ARGV[0].to_i
sieve = Array.new(limit + 1, false)
count = 0
(2..limit).each do |i|
  unless sieve[i]
    count += 1
    j = i * i
    while j <= limit
      sieve[j] = true
      j += i
    end
  end
end
puts count
