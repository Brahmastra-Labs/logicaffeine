n = ARGV[0].to_i
arr = []; seed = 42
n.times { seed=(seed*1103515245+12345)%2147483648; arr<<((seed>>16)&0x7fff)%n }
seen = {}; count = 0
arr.each { |x| c=n-x; count+=1 if c>=0 && seen[c]; seen[x]=true }
puts count
