import sys

def sieve(limit):
    is_composite = bytearray(limit + 1)
    count = 0
    h = 0
    for i in range(2, limit + 1):
        h = (h * 31 + (1 - is_composite[i])) & 268435455
        if not is_composite[i]:
            count += 1
            j = i * i
            while j <= limit:
                is_composite[j] = 1
                j += i
    return count, h

def main():
    limit = int(sys.argv[1])
    count, h = sieve(limit)
    print(count, h)

if __name__ == "__main__":
    main()
