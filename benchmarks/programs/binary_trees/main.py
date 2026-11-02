import sys
sys.setrecursionlimit(1000000)
class Node:
    __slots__ = ("left", "right")
    def __init__(self, left, right):
        self.left = left; self.right = right
def bottom_up_tree(d):
    if d > 0: return Node(bottom_up_tree(d-1), bottom_up_tree(d-1))
    return Node(None, None)
def check(node):
    if node.left is None: return 1
    return 1 + check(node.left) + check(node.right)
def main():
    n = int(sys.argv[1]); mn = 4; mx = max(mn+2, n)
    print(f"stretch tree of depth {mx+1}\t check: {check(bottom_up_tree(mx+1))}")
    ll = bottom_up_tree(mx)
    d = mn
    while d <= mx:
        it = 1 << (mx - d + mn); tc = 0
        for _ in range(it):
            tc += check(bottom_up_tree(d))
        print(f"{it}\t trees of depth {d}\t check: {tc}"); d += 2
    print(f"long lived tree of depth {mx}\t check: {check(ll)}")
if __name__ == "__main__":
    main()
