import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    b = s.encode()
    i = 0
    while i < len(b) and b[i] == 97:
        i += 1
    j = len(b)
    while j > i and b[j-1] == 97:
        j -= 1
    checksum += (j - i)
print(checksum)
