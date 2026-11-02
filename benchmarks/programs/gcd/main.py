import sys

def gcd(a, b):
    while b > 0:
        a, b = b, a % b
    return a


def main():
    n = int(sys.argv[1])
    s = 0
    for i in range(1, n + 1):
        for j in range(i, n + 1):
            s += gcd(i, j)
    print(s)


if __name__ == "__main__":
    main()
