import os, strutils
type Node = ref object
  left, right: Node
proc bottomUpTree(d: int): Node =
  if d > 0:
    let left = bottomUpTree(d - 1)
    let right = bottomUpTree(d - 1)
    Node(left: left, right: right)
  else: Node()
proc check(node: Node): int64 =
  if node.left == nil: return 1
  return 1 + check(node.left) + check(node.right)
proc main() =
  let n = parseInt(paramStr(1))
  let mn = 4; let mx = max(mn + 2, n)
  echo "stretch tree of depth ", mx+1, "\t check: ", check(bottomUpTree(mx+1))
  let ll = bottomUpTree(mx)
  var d = mn
  while d <= mx:
    let it = 1 shl (mx - d + mn)
    var tc: int64 = 0
    for i in 0..<it: tc += check(bottomUpTree(d))
    echo it, "\t trees of depth ", d, "\t check: ", tc
    d += 2
  echo "long lived tree of depth ", mx, "\t check: ", check(ll)

main()
