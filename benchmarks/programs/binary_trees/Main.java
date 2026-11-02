public class Main {
    static final class Node {
        final Node left, right;
        Node(Node left, Node right) { this.left = left; this.right = right; }
    }
    static Node bottomUpTree(int d) {
        if (d > 0) {
            Node left = bottomUpTree(d - 1);
            Node right = bottomUpTree(d - 1);
            return new Node(left, right);
        }
        return new Node(null, null);
    }
    static long check(Node node) { return node.left == null ? 1 : 1 + check(node.left) + check(node.right); }
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]), mn = 4, mx = Math.max(mn + 2, n);
        System.out.printf("stretch tree of depth %d\t check: %d%n", mx+1, check(bottomUpTree(mx+1)));
        Node ll = bottomUpTree(mx);
        for (int d = mn; d <= mx; d += 2) {
            int it = 1 << (mx - d + mn); long tc = 0;
            for (int i = 0; i < it; i++) tc += check(bottomUpTree(d));
            System.out.printf("%d\t trees of depth %d\t check: %d%n", it, d, tc);
        }
        System.out.printf("long lived tree of depth %d\t check: %d%n", mx, check(ll));
    }
}
