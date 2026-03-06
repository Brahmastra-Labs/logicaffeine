import sys
sys.setrecursionlimit(1000000)
def make_check(d):
    if d == 0: return 1
    return 1 + make_check(d-1) + make_check(d-1)
n = int(sys.argv[1]); mn = 4; mx = max(mn+2, n)
print(f"stretch tree of depth {mx+1}\t check: {make_check(mx+1)}")
ll = make_check(mx)
d = mn
while d <= mx:
    it = 1 << (mx - d + mn); tc = sum(make_check(d) for _ in range(it))
    print(f"{it}\t trees of depth {d}\t check: {tc}"); d += 2
print(f"long lived tree of depth {mx}\t check: {ll}")
