import sys
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += 64 - i.bit_length()   # leading zeros of i in a 64-bit word
print(s)
