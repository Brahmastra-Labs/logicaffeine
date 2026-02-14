def ackermann(m, n)
  return n + 1 if m == 0
  return ackermann(m - 1, 1) if n == 0
  ackermann(m - 1, ackermann(m, n - 1))
end

m = ARGV[0].to_i
puts ackermann(3, m)
