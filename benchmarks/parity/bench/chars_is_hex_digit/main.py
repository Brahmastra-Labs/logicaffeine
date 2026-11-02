import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    c = i % 128
    if (48 <= c <= 57) or (65 <= c <= 70) or (97 <= c <= 102):
        s += 1
print(s)
