import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    c = i % 128
    if (33 <= c <= 47) or (58 <= c <= 64) or (91 <= c <= 96) or (123 <= c <= 126):
        s += 1
print(s)
