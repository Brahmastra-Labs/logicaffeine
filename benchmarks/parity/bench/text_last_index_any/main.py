import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
sub = "XYZ"
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    idx = -1
    for i, ch in enumerate(s):
        if ch in sub:
            idx = i
    checksum += (idx + 1 if idx >= 0 else 0)
print(checksum)
