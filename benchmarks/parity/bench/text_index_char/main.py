import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    idx = s.find('0')
    checksum += (idx + 1 if idx >= 0 else 0)
print(checksum)
