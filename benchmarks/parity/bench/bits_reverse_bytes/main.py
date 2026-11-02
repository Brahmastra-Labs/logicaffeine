import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int.from_bytes((i & 0xFFFFFFFF).to_bytes(4,'big'),'little')
print(s)
