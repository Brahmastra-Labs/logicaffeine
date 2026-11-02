import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    idx = next((i for i, b in enumerate(s.encode()) if b == 48), -1)
    checksum += (idx + 1 if idx >= 0 else 0)
print(checksum)
