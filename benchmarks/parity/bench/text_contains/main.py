import sys
reps = int(sys.argv[1])
base = "abcABC012 " * 400
sub = "012"
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    if sub in s:
        checksum += 1
print(checksum)
