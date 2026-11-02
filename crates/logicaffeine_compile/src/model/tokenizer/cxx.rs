//! The libstdc++ behaviors llama.cpp's vocabulary inherits, ported step for step.
//!
//! [`sort`] is `std::sort`: llama.cpp orders its special-token cache by length alone, so tokens of equal length end
//! in the order this introsort leaves them, and that order decides which of two overlapping special tokens of one
//! length a text splits on first. [`unordered_map_order`] is the iteration order of a
//! `std::unordered_map<std::string, int>` filled one key at a time: llama.cpp takes the first token its map yields
//! that matches a list of texts as the end-of-turn and fill-in-the-middle tokens, so with two candidates (Qwen's
//! `<|endoftext|>` and `<|im_end|>`) the map's order picks one. `model_tokenizer_lock` holds both equal to the
//! library on this box.

const THRESHOLD: usize = 16;

/// `std::sort(v.begin(), v.end(), less)`.
pub fn sort<T: Copy>(v: &mut [T], less: &impl Fn(&T, &T) -> bool) {
    let n = v.len();
    if n == 0 {
        return;
    }
    let lg = (usize::BITS - 1 - n.leading_zeros()) as usize;
    introsort_loop(v, 0, n, 2 * lg, less);
    final_insertion_sort(v, 0, n, less);
}

fn introsort_loop<T: Copy>(v: &mut [T], first: usize, mut last: usize, mut depth: usize, less: &impl Fn(&T, &T) -> bool) {
    while last - first > THRESHOLD {
        if depth == 0 {
            heap_select(v, first, last, less);
            sort_heap(v, first, last, less);
            return;
        }
        depth -= 1;
        let mid = first + (last - first) / 2;
        move_median_to_first(v, first, first + 1, mid, last - 1, less);
        let cut = unguarded_partition(v, first + 1, last, first, less);
        introsort_loop(v, cut, last, depth, less);
        last = cut;
    }
}

fn move_median_to_first<T: Copy>(v: &mut [T], result: usize, a: usize, b: usize, c: usize, less: &impl Fn(&T, &T) -> bool) {
    if less(&v[a], &v[b]) {
        if less(&v[b], &v[c]) {
            v.swap(result, b);
        } else if less(&v[a], &v[c]) {
            v.swap(result, c);
        } else {
            v.swap(result, a);
        }
    } else if less(&v[a], &v[c]) {
        v.swap(result, a);
    } else if less(&v[b], &v[c]) {
        v.swap(result, c);
    } else {
        v.swap(result, b);
    }
}

fn unguarded_partition<T: Copy>(v: &mut [T], mut first: usize, mut last: usize, pivot: usize, less: &impl Fn(&T, &T) -> bool) -> usize {
    loop {
        while less(&v[first], &v[pivot]) {
            first += 1;
        }
        last -= 1;
        while less(&v[pivot], &v[last]) {
            last -= 1;
        }
        if first >= last {
            return first;
        }
        v.swap(first, last);
        first += 1;
    }
}

fn adjust_heap<T: Copy>(v: &mut [T], first: usize, mut hole: usize, len: usize, value: T, less: &impl Fn(&T, &T) -> bool) {
    let top = hole;
    let mut second = hole;
    while second < (len - 1) / 2 {
        second = 2 * (second + 1);
        if less(&v[first + second], &v[first + second - 1]) {
            second -= 1;
        }
        v[first + hole] = v[first + second];
        hole = second;
    }
    if len & 1 == 0 && second == (len - 2) / 2 {
        second = 2 * (second + 1);
        v[first + hole] = v[first + second - 1];
        hole = second - 1;
    }
    let mut parent = hole.wrapping_sub(1) / 2;
    while hole > top && less(&v[first + parent], &value) {
        v[first + hole] = v[first + parent];
        hole = parent;
        parent = hole.wrapping_sub(1) / 2;
    }
    v[first + hole] = value;
}

fn make_heap<T: Copy>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    let len = last - first;
    if len < 2 {
        return;
    }
    let mut parent = (len - 2) / 2;
    loop {
        let value = v[first + parent];
        adjust_heap(v, first, parent, len, value, less);
        if parent == 0 {
            return;
        }
        parent -= 1;
    }
}

fn pop_heap<T: Copy>(v: &mut [T], first: usize, last: usize, result: usize, less: &impl Fn(&T, &T) -> bool) {
    let value = v[result];
    v[result] = v[first];
    adjust_heap(v, first, 0, last - first, value, less);
}

fn heap_select<T: Copy>(v: &mut [T], first: usize, middle: usize, less: &impl Fn(&T, &T) -> bool) {
    make_heap(v, first, middle, less);
}

fn sort_heap<T: Copy>(v: &mut [T], first: usize, mut last: usize, less: &impl Fn(&T, &T) -> bool) {
    while last - first > 1 {
        last -= 1;
        pop_heap(v, first, last, last, less);
    }
}

fn final_insertion_sort<T: Copy>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    if last - first > THRESHOLD {
        insertion_sort(v, first, first + THRESHOLD, less);
        for i in first + THRESHOLD..last {
            unguarded_linear_insert(v, i, less);
        }
    } else {
        insertion_sort(v, first, last, less);
    }
}

fn insertion_sort<T: Copy>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    if first == last {
        return;
    }
    for i in first + 1..last {
        if less(&v[i], &v[first]) {
            let val = v[i];
            v.copy_within(first..i, first + 1);
            v[first] = val;
        } else {
            unguarded_linear_insert(v, i, less);
        }
    }
}

fn unguarded_linear_insert<T: Copy>(v: &mut [T], mut last: usize, less: &impl Fn(&T, &T) -> bool) {
    let val = v[last];
    let mut next = last - 1;
    while less(&val, &v[next]) {
        v[last] = v[next];
        last = next;
        next -= 1;
    }
    v[last] = val;
}

/// `std::_Hash_bytes` for 64-bit `size_t` (MurmurHash64A over unaligned little-endian words), the hash
/// `std::hash<std::string>` runs with seed `0xc70f6907`.
pub fn hash_bytes(b: &[u8], seed: u64) -> u64 {
    const MUL: u64 = (0xc6a4a793u64 << 32) + 0x5bd1e995;
    let shift_mix = |v: u64| v ^ (v >> 47);
    let aligned = b.len() & !7;
    let mut hash = seed ^ (b.len() as u64).wrapping_mul(MUL);
    for w in b[..aligned].chunks_exact(8) {
        let data = shift_mix(u64::from_le_bytes(w.try_into().expect("an 8-byte word")).wrapping_mul(MUL)).wrapping_mul(MUL);
        hash ^= data;
        hash = hash.wrapping_mul(MUL);
    }
    if b.len() & 7 != 0 {
        let data = b[aligned..].iter().rev().fold(0u64, |acc, &c| (acc << 8) + c as u64);
        hash ^= data;
        hash = hash.wrapping_mul(MUL);
    }
    hash = shift_mix(hash).wrapping_mul(MUL);
    shift_mix(hash)
}

/// `std::hash<std::string>`.
pub fn hash_string(b: &[u8]) -> u64 {
    hash_bytes(b, 0xc70f6907)
}

/// `__prime_list` past its first six entries, the bucket counts `_Prime_rehash_policy::_M_next_bkt` chooses among;
/// the last is the sentinel past which no table grows.
const PRIMES: [u64; 298] = [
    17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97, 103, 109, 113, 127, 137, 139, 149,
    157, 167, 179, 193, 199, 211, 227, 241, 257, 277, 293, 313, 337, 359, 383, 409, 439, 467, 503, 541, 577, 619,
    661, 709, 761, 823, 887, 953, 1031, 1109, 1193, 1289, 1381, 1493, 1613, 1741, 1879, 2029, 2179, 2357, 2549,
    2753, 2971, 3209, 3469, 3739, 4027, 4349, 4703, 5087, 5503, 5953, 6427, 6949, 7517, 8123, 8783, 9497, 10273,
    11113, 12011, 12983, 14033, 15173, 16411, 17749, 19183, 20753, 22447, 24281, 26267, 28411, 30727, 33223, 35933,
    38873, 42043, 45481, 49201, 53201, 57557, 62233, 67307, 72817, 78779, 85229, 92203, 99733, 107897, 116731,
    126271, 136607, 147793, 159871, 172933, 187091, 202409, 218971, 236897, 256279, 277261, 299951, 324503, 351061,
    379787, 410857, 444487, 480881, 520241, 562841, 608903, 658753, 712697, 771049, 834181, 902483, 976369,
    1056323, 1142821, 1236397, 1337629, 1447153, 1565659, 1693859, 1832561, 1982627, 2144977, 2320627, 2510653,
    2716249, 2938679, 3179303, 3439651, 3721303, 4026031, 4355707, 4712381, 5098259, 5515729, 5967347, 6456007,
    6984629, 7556579, 8175383, 8844859, 9569143, 10352717, 11200489, 12117689, 13109983, 14183539, 15345007,
    16601593, 17961079, 19431899, 21023161, 22744717, 24607243, 26622317, 28802401, 31160981, 33712729, 36473443,
    39460231, 42691603, 46187573, 49969847, 54061849, 58488943, 63278561, 68460391, 74066549, 80131819, 86693767,
    93793069, 101473717, 109783337, 118773397, 128499677, 139022417, 150406843, 162723577, 176048909, 190465427,
    206062531, 222936881, 241193053, 260944219, 282312799, 305431229, 330442829, 357502601, 386778277, 418451333,
    452718089, 489790921, 529899637, 573292817, 620239453, 671030513, 725980837, 785430967, 849749479, 919334987,
    994618837, 1076067617, 1164186217, 1259520799, 1362662261, 1474249943, 1594975441, 1725587117, 1866894511,
    2019773507, 2185171673, 2364114217, 2557710269, 2767159799, 2993761039, 3238918481, 3504151727, 3791104843,
    4101556399, 4294967291, 6442450933, 8589934583, 12884901857, 17179869143, 25769803693, 34359738337,
    51539607367, 68719476731, 103079215087, 137438953447, 206158430123, 274877906899, 412316860387, 549755813881,
    824633720731, 1099511627689, 1649267441579, 2199023255531, 3298534883309, 4398046511093, 6597069766607,
    8796093022151, 13194139533241, 17592186044399, 26388279066581, 35184372088777, 52776558133177, 70368744177643,
    105553116266399, 140737488355213, 211106232532861, 281474976710597, 562949953421231, 1125899906842597,
    2251799813685119, 4503599627370449, 9007199254740881, 18014398509481951, 36028797018963913, 72057594037927931,
    144115188075855859, 288230376151711717, 576460752303423433, 1152921504606846883, 2305843009213693951,
    4611686018427387847, 9223372036854775783, 18446744073709551557,
];

/// `_Prime_rehash_policy` at its default maximum load factor of 1.
struct RehashPolicy {
    next_resize: u64,
}

impl RehashPolicy {
    fn next_bkt(&mut self, n: u64) -> u64 {
        const FAST: [u64; 14] = [2, 2, 2, 3, 5, 5, 7, 7, 11, 11, 11, 11, 13, 13];
        if n < FAST.len() as u64 {
            if n == 0 {
                return 1;
            }
            self.next_resize = FAST[n as usize];
            return FAST[n as usize];
        }
        let last = PRIMES.len() - 1;
        let i = PRIMES[..last].partition_point(|&p| p < n);
        self.next_resize = if i == last { u64::MAX } else { PRIMES[i] };
        PRIMES[i]
    }

    fn need_rehash(&mut self, n_bkt: u64, n_elt: u64, n_ins: u64) -> Option<u64> {
        if n_elt + n_ins <= self.next_resize {
            return None;
        }
        let min_bkts = (n_elt + n_ins).max(if self.next_resize == 0 { 11 } else { 0 }) as f64;
        if min_bkts >= n_bkt as f64 {
            return Some(self.next_bkt((min_bkts.floor() as u64 + 1).max(n_bkt * 2)));
        }
        self.next_resize = n_bkt;
        None
    }
}

/// The order a `std::unordered_map` iterates keys inserted in the order of `hashes` (each key distinct): libstdc++'s
/// `_Hashtable` with cached hash codes, a new node at the front of its bucket (or of the whole list when its bucket
/// is empty), and the unique-key rehash that relinks nodes front-first. The result lists insertion indices.
pub fn unordered_map_order(hashes: &[u64]) -> Vec<u32> {
    const NIL: u32 = u32::MAX;
    const EMPTY: i64 = -1;
    const BEFORE_BEGIN: i64 = -2;
    let mut next = vec![NIL; hashes.len()];
    let mut begin = NIL;
    let mut policy = RehashPolicy { next_resize: 0 };
    let mut bucket_count: u64 = 1;
    let mut buckets: Vec<i64> = vec![EMPTY];
    let get = |b: i64, begin: u32, next: &[u32]| if b == BEFORE_BEGIN { begin } else { next[b as usize] };
    for (node, &h) in hashes.iter().enumerate() {
        let node = node as u32;
        if let Some(n) = policy.need_rehash(bucket_count, node as u64, 1) {
            let mut fresh = vec![EMPTY; n as usize];
            let mut p = begin;
            begin = NIL;
            let mut bbegin = 0usize;
            while p != NIL {
                let after = next[p as usize];
                let bkt = (hashes[p as usize] % n) as usize;
                if fresh[bkt] == EMPTY {
                    next[p as usize] = begin;
                    begin = p;
                    fresh[bkt] = BEFORE_BEGIN;
                    if next[p as usize] != NIL {
                        fresh[bbegin] = p as i64;
                    }
                    bbegin = bkt;
                } else {
                    let b = fresh[bkt];
                    next[p as usize] = get(b, begin, &next);
                    if b == BEFORE_BEGIN {
                        begin = p;
                    } else {
                        next[b as usize] = p;
                    }
                }
                p = after;
            }
            buckets = fresh;
            bucket_count = n;
        }
        let bkt = (h % bucket_count) as usize;
        if buckets[bkt] != EMPTY {
            let b = buckets[bkt];
            next[node as usize] = get(b, begin, &next);
            if b == BEFORE_BEGIN {
                begin = node;
            } else {
                next[b as usize] = node;
            }
        } else {
            next[node as usize] = begin;
            begin = node;
            let after = next[node as usize];
            if after != NIL {
                buckets[(hashes[after as usize] % bucket_count) as usize] = node as i64;
            }
            buckets[bkt] = BEFORE_BEGIN;
        }
    }
    let mut order = Vec::with_capacity(hashes.len());
    let mut p = begin;
    while p != NIL {
        order.push(p);
        p = next[p as usize];
    }
    order
}
