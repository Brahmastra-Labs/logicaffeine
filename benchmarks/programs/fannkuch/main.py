import sys

def main():
    n = int(sys.argv[1])
    perm1 = list(range(n))
    perm = [0] * n
    count = [0] * n
    max_flips = checksum = perm_count = 0
    r = n
    while True:
        while r > 1: count[r-1] = r; r -= 1
        perm[:] = perm1
        flips = 0
        while perm[0] != 0:
            k = perm[0] + 1
            for i in range(k // 2):
                perm[i], perm[k-1-i] = perm[k-1-i], perm[i]
            flips += 1
        if flips > max_flips: max_flips = flips
        checksum += flips if perm_count % 2 == 0 else -flips
        perm_count += 1
        while True:
            if r == n:
                print(f"{checksum}\n{max_flips}")
                return
            p0 = perm1[0]
            for i in range(r):
                perm1[i] = perm1[i+1]
            perm1[r] = p0
            count[r] -= 1
            if count[r] > 0: break
            r += 1

if __name__ == "__main__":
    main()
