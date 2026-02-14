import sys

def sieve(limit):
    is_composite = bytearray(limit + 1)
    count = 0
    for i in range(2, limit + 1):
        if not is_composite[i]:
            count += 1
            j = i * i
            while j <= limit:
                is_composite[j] = 1
                j += i
    return count

limit = int(sys.argv[1])
print(sieve(limit))
