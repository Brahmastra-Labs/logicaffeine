import sys


def main():
    n = int(sys.argv[1])
    arr = [(i * 7 + 3) % 1000000 for i in range(n)]
    h = 0
    for v in arr:
        h = (h * 31 + v) & 268435455
    print(h)


if __name__ == "__main__":
    main()
