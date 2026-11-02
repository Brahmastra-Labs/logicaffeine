class Node { constructor(left, right) { this.left = left; this.right = right; } }
function bottomUpTree(d) { return d > 0 ? new Node(bottomUpTree(d-1), bottomUpTree(d-1)) : new Node(null, null); }
function check(node) { return node.left === null ? 1 : 1 + check(node.left) + check(node.right); }
const n = parseInt(process.argv[2]), mn = 4;
let mx = Math.max(mn + 2, n);
console.log(`stretch tree of depth ${mx+1}\t check: ${check(bottomUpTree(mx+1))}`);
const ll = bottomUpTree(mx);
for (let d = mn; d <= mx; d += 2) {
    const it = 1 << (mx - d + mn); let tc = 0;
    for (let i = 0; i < it; i++) tc += check(bottomUpTree(d));
    console.log(`${it}\t trees of depth ${d}\t check: ${tc}`);
}
console.log(`long lived tree of depth ${mx}\t check: ${check(ll)}`);
