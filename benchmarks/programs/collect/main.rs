// collect — the table main.c hand-rolls: flat 16-byte (key, value) slots, a power-of-two capacity at least twice n,
// Fibonacci hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory
// and the key 0 itself kept beside the table.

struct Table {
    slots: Vec<[i64; 2]>,
    mask: usize,
    shift: u32,
    has_zero: bool,
    zero_value: i64,
}

impl Table {
    fn new(n: i64) -> Table {
        let mut bits: u32 = 4;
        while (1i64 << bits) < 2 * n {
            bits += 1;
        }
        let capacity = 1usize << bits;
        Table {
            slots: vec![[0; 2]; capacity],
            mask: capacity - 1,
            shift: 64 - bits,
            has_zero: false,
            zero_value: 0,
        }
    }

    #[inline]
    fn slot_of(&self, key: i64) -> usize {
        ((key as u64).wrapping_mul(0x9E3779B97F4A7C15) >> self.shift) as usize
    }

    fn insert(&mut self, key: i64, value: i64) {
        if key == 0 {
            self.has_zero = true;
            self.zero_value = value;
            return;
        }
        let mut i = self.slot_of(key);
        while self.slots[i][0] != 0 && self.slots[i][0] != key {
            i = (i + 1) & self.mask;
        }
        self.slots[i] = [key, value];
    }

    fn find(&self, key: i64) -> Option<i64> {
        if key == 0 {
            return if self.has_zero {
                Some(self.zero_value)
            } else {
                None
            };
        }
        let mut i = self.slot_of(key);
        while self.slots[i][0] != 0 {
            if self.slots[i][0] == key {
                return Some(self.slots[i][1]);
            }
            i = (i + 1) & self.mask;
        }
        None
    }
}

fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut table = Table::new(n);
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        table.insert(seed, seed * 2);
    }
    let mut hit: i64 = 42;
    let mut miss: i64 = seed;
    let mut found: i64 = 0;
    let mut h: i64 = 0;
    for _ in 0..n {
        hit = (hit.wrapping_mul(1103515245) + 12345) % 2147483648;
        if let Some(v) = table.find(hit) {
            found += 1;
            h = (h * 31 + v) & 268435455;
        }
        miss = (miss.wrapping_mul(1103515245) + 12345) % 2147483648;
        if let Some(v) = table.find(miss) {
            found += 1;
            h = (h * 31 + v) & 268435455;
        }
    }
    println!("{} {}", found, h);
}
