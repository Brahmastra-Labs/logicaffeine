import sys
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 256 for k in range(M)]
sm = 0
for r in range(reps):
    sm += xs[r % M]
print(sm)
