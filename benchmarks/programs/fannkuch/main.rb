n = ARGV[0].to_i
perm1 = (0...n).to_a
count = Array.new(n, 0)
max_flips = checksum = perm_count = 0
r = n
loop do
  while r > 1; count[r-1] = r; r -= 1; end
  perm = perm1.dup
  flips = 0
  while perm[0] != 0
    k = perm[0] + 1
    perm[0, k] = perm[0, k].reverse
    flips += 1
  end
  max_flips = flips if flips > max_flips
  checksum += perm_count.even? ? flips : -flips
  perm_count += 1
  loop do
    if r == n; puts checksum; puts max_flips; exit; end
    p0 = perm1[0]
    (0...r).each { |i| perm1[i] = perm1[i+1] }
    perm1[r] = p0
    count[r] -= 1
    break if count[r] > 0
    r += 1
  end
end
