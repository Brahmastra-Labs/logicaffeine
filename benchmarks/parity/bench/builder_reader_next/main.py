import sys
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 256 for k in range(M)]
sm = 0
for r in range(reps):
    pos = r % (M-10)
    sm += sum(xs[pos:pos+10])
print(sm)
