import sys

n = int(sys.argv[1])
parts = []
for i in range(n):
    parts.append(str(i))
    parts.append(' ')
result = ''.join(parts)
spaces = result.count(' ')
print(spaces)
