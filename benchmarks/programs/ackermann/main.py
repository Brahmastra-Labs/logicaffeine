import sys

sys.setrecursionlimit(100000)

def ackermann(m, n):
    if m == 0:
        return n + 1
    if n == 0:
        return ackermann(m - 1, 1)
    return ackermann(m - 1, ackermann(m, n - 1))


def main():
    m = int(sys.argv[1])
    print(ackermann(3, m))


if __name__ == "__main__":
    main()
