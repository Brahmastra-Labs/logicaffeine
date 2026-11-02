n = ARGV[0].to_i
text = []
seed = 42
n.times do
  seed = (seed * 1103515245 + 12345) % 2147483648
  text << (97 + (seed >> 16) % 5).chr
end
text = text.join
needle = "cabed"
needle_len = 5
count = 0
positions = []
i = 0
while i <= n - needle_len
  match = true
  j = 0
  while j < needle_len
    if text.getbyte(i + j) != needle.getbyte(j)
      match = false
      break
    end
    j += 1
  end
  if match
    count += 1
    positions << i
  end
  i += 1
end
checksum = 0
positions.each { |p| checksum = (checksum * 31 + p) & 268435455 }
puts "#{count} #{checksum}"
