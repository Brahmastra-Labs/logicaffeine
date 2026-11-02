import sys


def main():
    n = int(sys.argv[1])
    capacity = n * 5
    weight = [0] * n
    value = [0] * n
    seed = 42
    for i in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        weight[i] = (seed >> 16) % 50 + 1
        seed = (seed * 1103515245 + 12345) % 2147483648
        value[i] = (seed >> 16) % 100 + 1
    prev = [0] * (capacity + 1)
    curr = [0] * (capacity + 1)
    for i in range(n):
        w = weight[i]
        v = value[i]
        for j in range(capacity + 1):
            curr[j] = prev[j]
            if j >= w and prev[j - w] + v > curr[j]:
                curr[j] = prev[j - w] + v
        prev, curr = curr, prev
    h = 0
    for j in range(capacity + 1):
        h = (h * 31 + prev[j]) & 268435455
    print(prev[capacity], h)


if __name__ == "__main__":
    main()
