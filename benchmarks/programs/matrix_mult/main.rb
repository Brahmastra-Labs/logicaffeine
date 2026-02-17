MOD = 1000000007
n = ARGV[0].to_i
a = Array.new(n * n, 0)
b = Array.new(n * n, 0)
c = Array.new(n * n, 0)
(0...n).each{|i| (0...n).each{|j|
  a[i*n+j] = (i*n+j) % 100
  b[i*n+j] = (j*n+i) % 100
}}
(0...n).each{|i| (0...n).each{|k| (0...n).each{|j|
  c[i*n+j] = (c[i*n+j] + a[i*n+k] * b[k*n+j]) % MOD
}}}
checksum = 0
c.each{|v| checksum = (checksum + v) % MOD }
puts checksum
