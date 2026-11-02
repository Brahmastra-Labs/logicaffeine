import sys


def main():
    n = int(sys.argv[1])
    parts = []
    for i in range(n):
        parts.append(str(i))
        parts.append(' ')
    result = ''.join(parts)
    spaces = 0
    for c in result:
        if c == ' ':
            spaces += 1
    h = 0
    for b in result.encode():
        h = (h * 31 + b) & 268435455
    print(spaces, h)


if __name__ == "__main__":
    main()
