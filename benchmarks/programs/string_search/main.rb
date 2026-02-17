n = ARGV[0].to_i
text = []
pos = 0
while pos < n
  if pos > 0 && pos % 1000 == 0 && pos + 5 <= n
    text << "XXXXX"
    pos += 5
  else
    text << (97 + pos % 5).chr
    pos += 1
  end
end
text = text.join
needle = "XXXXX"
count = 0
i = 0
while (idx = text.index(needle, i))
  count += 1
  i = idx + 1
end
puts count
