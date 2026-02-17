import sys
n = int(sys.argv[1])
perm1 = list(range(n))
count = [0] * n
max_flips = checksum = perm_count = 0
r = n
while True:
    while r > 1: count[r-1] = r; r -= 1
    perm = perm1[:]
    flips = 0
    while perm[0] != 0:
        k = perm[0] + 1
        perm[:k] = perm[:k][::-1]
        flips += 1
    if flips > max_flips: max_flips = flips
    checksum += flips if perm_count % 2 == 0 else -flips
    perm_count += 1
    while True:
        if r == n: print(f"{checksum}\n{max_flips}"); sys.exit()
        p0 = perm1[0]
        perm1[:r] = perm1[1:r+1]
        perm1[r] = p0
        count[r] -= 1
        if count[r] > 0: break
        r += 1
