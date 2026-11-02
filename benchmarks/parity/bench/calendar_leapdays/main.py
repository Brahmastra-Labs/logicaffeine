import sys, calendar
n = int(sys.argv[1])
s = 0
for i in range(n):
    a = 1800 + (i % 400)
    s += calendar.leapdays(a, a + 40)
print(s)
