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
count = 0
i = 0
while True:
    idx = text.find(needle, i)
    if idx == -1:
        break
    count += 1
    i = idx + 1
print(count)
