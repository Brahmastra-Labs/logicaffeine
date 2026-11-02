import sys


def main():
    n = int(sys.argv[1])
    counts = [0] * 1000
    seed = 42
    for _ in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        counts[((seed >> 16) & 0x7fff) % 1000] += 1
    max_freq = max_idx = 0
    for i in range(1000):
        if counts[i] > max_freq: max_freq = counts[i]; max_idx = i
    h = 0
    for c in counts:
        h = (h * 31 + c) & 268435455
    print(f"{max_freq} {max_idx} {h}")


if __name__ == "__main__":
    main()
