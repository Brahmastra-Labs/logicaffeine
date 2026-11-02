use std::env;
struct Node { left: Option<Box<Node>>, right: Option<Box<Node>> }
fn bottom_up_tree(d: i32) -> Box<Node> {
    if d > 0 {
        Box::new(Node { left: Some(bottom_up_tree(d-1)), right: Some(bottom_up_tree(d-1)) })
    } else {
        Box::new(Node { left: None, right: None })
    }
}
fn check(node: &Node) -> i64 {
    match (&node.left, &node.right) {
        (Some(l), Some(r)) => 1 + check(l) + check(r),
        _ => 1,
    }
}
fn main() {
    let n: i32 = env::args().nth(1).unwrap().parse().unwrap();
    let mn = 4; let mx = if mn + 2 > n { mn + 2 } else { n };
    let stretch = bottom_up_tree(mx+1);
    println!("stretch tree of depth {}\t check: {}", mx+1, check(&stretch));
    drop(stretch);
    let ll = bottom_up_tree(mx);
    let mut d = mn;
    while d <= mx {
        let it = 1 << (mx - d + mn);
        let tc: i64 = (0..it).map(|_| check(&bottom_up_tree(d))).sum();
        println!("{}\t trees of depth {}\t check: {}", it, d, tc);
        d += 2;
    }
    println!("long lived tree of depth {}\t check: {}", mx, check(&ll));
}
