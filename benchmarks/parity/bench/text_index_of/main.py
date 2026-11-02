import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
sub = "012"
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    checksum += (s.find(sub) + 1 if sub in s else 0)
print(checksum)
