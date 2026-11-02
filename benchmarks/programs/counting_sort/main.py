import sys


def main():
    n = int(sys.argv[1])
    arr = []
    seed = 42
    for _ in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        arr.append((seed >> 16) % 1000)
    counts = [0] * 1000
    for v in arr: counts[v] += 1
    sorted_arr = []
    for v in range(1000):
        for _ in range(counts[v]):
            sorted_arr.append(v)
    checksum = 0
    for v in sorted_arr: checksum = (checksum * 31 + v) & 268435455
    print(f"{sorted_arr[0]} {sorted_arr[-1]} {checksum}")


if __name__ == "__main__":
    main()
