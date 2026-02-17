import sys
n = int(sys.argv[1])
count = 0
for y in range(n):
    for x in range(n):
        cr = 2.0*x/n - 1.5; ci = 2.0*y/n - 1.0; zr = zi = 0.0; inside = True
        for _ in range(50):
            zr, zi = zr*zr - zi*zi + cr, 2*zr*zi + ci
            if zr*zr + zi*zi > 4: inside = False; break
        if inside: count += 1
print(count)
