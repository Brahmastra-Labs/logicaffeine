MOD = 1000000007
n = ARGV[0].to_i
a = Array.new(n * n, 0)
b = Array.new(n * n, 0)
c = Array.new(n * n, 0)
seed = 42
(0...n*n).each{|i|
  seed = (seed * 1103515245 + 12345) % 2147483648
  a[i] = (seed >> 16) % 100
}
(0...n*n).each{|i|
  seed = (seed * 1103515245 + 12345) % 2147483648
  b[i] = (seed >> 16) % 100
}
(0...n).each{|i| (0...n).each{|k| (0...n).each{|j|
  c[i*n+j] = (c[i*n+j] + a[i*n+k] * b[k*n+j]) % MOD
}}}
checksum = 0
c.each{|v| checksum = (checksum * 31 + v) & 268435455 }
puts checksum
