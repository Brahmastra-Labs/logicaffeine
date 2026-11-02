import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    parts = s.split("0", 1)
    checksum += (len(parts[1]) if len(parts) > 1 else 0)
print(checksum)
