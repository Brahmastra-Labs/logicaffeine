limit = ARGV[0].to_i
sieve = Array.new(limit + 1, false)
count = 0
h = 0
(2..limit).each do |i|
  h = (h * 31 + (sieve[i] ? 0 : 1)) & 268435455
  unless sieve[i]
    count += 1
    j = i * i
    while j <= limit
      sieve[j] = true
      j += i
    end
  end
end
puts "#{count} #{h}"
