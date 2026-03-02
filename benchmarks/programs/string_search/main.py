import sys
n = int(sys.argv[1])
text = []
pos = 0
while pos < n:
    if pos > 0 and pos % 1000 == 0 and pos + 5 <= n:
        text.append("XXXXX")
        pos += 5
    else:
        text.append(chr(97 + pos % 5))
        pos += 1
text = ''.join(text)
needle = "XXXXX"
needle_len = 5
count = 0
for i in range(n - needle_len + 1):
    match = True
    for j in range(needle_len):
        if text[i + j] != needle[j]:
            match = False
            break
    if match:
        count += 1
print(count)
