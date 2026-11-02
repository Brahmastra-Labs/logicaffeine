import sys
import base64
reps = int(sys.argv[1])
base = "abcABC012 " * 400
checksum = 0
for r in range(reps):
    s = "a" * (r % 100) + base
    checksum += len(base64.b32encode(s.encode()))
print(checksum)
