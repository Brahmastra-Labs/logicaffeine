import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
sub = "XYZ"
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    if any(ch in sub for ch in s):
        checksum += 1
print(checksum)
