n = ARGV[0].to_i
result = ""
n.times { |i| result << i.to_s << " " }
spaces = result.count(" ")
puts spaces
