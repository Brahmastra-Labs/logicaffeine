n = ARGV[0].to_i
coins = [1, 5, 10, 25, 50, 100]
dp = Array.new(n + 1, 0)
dp[0] = 1
coins.each { |c| (c..n).each { |j| dp[j] = (dp[j] + dp[j - c]) % 1000000007 } }
checksum = 0
dp.each { |v| checksum = (checksum * 31 + v) & 268435455 }
puts "#{dp[n]} #{checksum}"
