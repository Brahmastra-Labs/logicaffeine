import sys


def main():
    n = int(sys.argv[1])
    arr = []
    seed = 42
    for _ in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        arr.append(((seed >> 16) & 0x7fff) % 1000)
    for i in range(1, n):
        arr[i] = (arr[i] + arr[i-1]) % 1000000007
    h = 0
    for v in arr:
        h = (h * 31 + v) & 268435455
    print(arr[-1], h)


if __name__ == "__main__":
    main()
