def solve(row, cols, diag1, diag2, n)
  return 1 if row == n
  count = 0
  available = ((1 << n) - 1) & ~(cols | diag1 | diag2)
  while available != 0
    bit = available & (-available)
    available ^= bit
    count += solve(row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1, n)
  end
  count
end

n = ARGV[0].to_i
puts solve(0, 0, 0, 0, n)
