import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    checksum += len(bytes((b + 1) % 128 for b in s.encode()))
print(checksum)
