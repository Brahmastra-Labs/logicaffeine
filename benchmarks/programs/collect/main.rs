use std::collections::HashMap;

fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut map = HashMap::with_capacity(n as usize);
    for i in 0..n {
        map.insert(i, i * 2);
    }
    let mut found: i64 = 0;
    for i in 0..n {
        if map.get(&i) == Some(&(i * 2)) {
            found += 1;
        }
    }
    println!("{}", found);
}
