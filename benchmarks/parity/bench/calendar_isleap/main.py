import sys
n = int(sys.argv[1])


def leap(y):
    if y % 400 == 0:
        return True
    if y % 100 == 0:
        return False
    return y % 4 == 0


s = 0
for i in range(1, n + 1):
    if leap(i):
        s += 1
print(s)
