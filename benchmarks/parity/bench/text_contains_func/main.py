import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    if any(b > 200 for b in s.encode()):
        checksum += 1
print(checksum)
