// two_sum — the set main.c hand-rolls: flat 8-byte key slots, a power-of-two capacity at least twice n, Fibonacci
// hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory and the key
// 0 itself a flag beside the table.

struct Set {
    slots: Vec<i64>,
    mask: usize,
    shift: u32,
    has_zero: bool,
}

impl Set {
    fn new(n: i64) -> Set {
        let mut bits: u32 = 4;
        while (1i64 << bits) < 2 * n {
            bits += 1;
        }
        let capacity = 1usize << bits;
        Set {
            slots: vec![0; capacity],
            mask: capacity - 1,
            shift: 64 - bits,
            has_zero: false,
        }
    }

    #[inline]
    fn slot_of(&self, key: i64) -> usize {
        ((key as u64).wrapping_mul(0x9E3779B97F4A7C15) >> self.shift) as usize
    }

    fn contains(&self, key: i64) -> bool {
        if key == 0 {
            return self.has_zero;
        }
        let mut h = self.slot_of(key);
        while self.slots[h] != 0 {
            if self.slots[h] == key {
                return true;
            }
            h = (h + 1) & self.mask;
        }
        false
    }

    fn insert(&mut self, key: i64) {
        if key == 0 {
            self.has_zero = true;
            return;
        }
        let mut h = self.slot_of(key);
        while self.slots[h] != 0 {
            if self.slots[h] == key {
                return;
            }
            h = (h + 1) & self.mask;
        }
        self.slots[h] = key;
    }
}

fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let target: i64 = 2147483647;
    let mut arr = Vec::with_capacity(n as usize);
    let mut seed: i64 = 42;
    for i in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        let v = if i > 0 && (seed >> 28) == 0 {
            target - arr[(seed % i) as usize]
        } else {
            seed
        };
        arr.push(v);
    }
    let mut seen = Set::new(n);
    let mut count: i64 = 0;
    for &x in &arr {
        if seen.contains(target - x) {
            count += 1;
        }
        seen.insert(x);
    }
    println!("{}", count);
}
