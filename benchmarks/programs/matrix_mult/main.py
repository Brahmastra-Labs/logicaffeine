import sys
MOD = 1000000007

def main():
    n = int(sys.argv[1])
    a = [0] * (n * n)
    b = [0] * (n * n)
    c = [0] * (n * n)
    seed = 42
    for i in range(n * n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        a[i] = (seed >> 16) % 100
    for i in range(n * n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        b[i] = (seed >> 16) % 100
    for i in range(n):
        for k in range(n):
            for j in range(n):
                c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) % MOD
    checksum = 0
    for v in c:
        checksum = (checksum * 31 + v) & 268435455
    print(checksum)

if __name__ == "__main__":
    main()
