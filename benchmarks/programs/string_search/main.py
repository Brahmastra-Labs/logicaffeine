import sys


def main():
    n = int(sys.argv[1])
    text = []
    seed = 42
    for pos in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        text.append(chr(97 + (seed >> 16) % 5))
    text = ''.join(text)
    needle = "cabed"
    needle_len = 5
    count = 0
    positions = []
    for i in range(n - needle_len + 1):
        match = True
        for j in range(needle_len):
            if text[i + j] != needle[j]:
                match = False
                break
        if match:
            count += 1
            positions.append(i)
    checksum = 0
    for p in positions: checksum = (checksum * 31 + p) & 268435455
    print(f"{count} {checksum}")


if __name__ == "__main__":
    main()
