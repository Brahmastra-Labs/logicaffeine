import sys

n = int(sys.argv[1])
total = 0
for i in range(1, n + 1):
    k = i
    while k != 1:
        if k % 2 == 0:
            k //= 2
        else:
            k = 3 * k + 1
        total += 1
print(total)
