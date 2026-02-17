import sys

n = int(sys.argv[1])
count = 0
for i in range(2, n + 1):
    is_prime = True
    d = 2
    while d * d <= i:
        if i % d == 0:
            is_prime = False
            break
        d += 1
    if is_prime:
        count += 1
print(count)
