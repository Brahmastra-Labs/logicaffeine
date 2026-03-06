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
needle_len = 5
count = 0
i = 0
while i <= n - needle_len
  match = true
  j = 0
  while j < needle_len
    if text[i + j] != needle[j]
      match = false
      break
    end
    j += 1
  end
  count += 1 if match
  i += 1
end
puts count
