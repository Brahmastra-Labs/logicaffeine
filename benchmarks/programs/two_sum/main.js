// two_sum — the set main.c hand-rolls: flat key slots, here a Float64Array (doubles hold these keys exactly), a
// power-of-two capacity at least twice n, Fibonacci hashing (JavaScript has no 64-bit multiply, so the key times
// 2^32/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory and the key 0 itself a flag beside
// the table. The input array is a presized Float64Array as main.c's is a presized long[].
const n = parseInt(process.argv[2]);
const target = 2147483647;
const arr = new Float64Array(n);
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    arr[i] = (i > 0 && (seed >> 28) === 0) ? target - arr[seed % i] : seed;
}
let bits = 4;
while (2 ** bits < 2 * n) bits++;
const capacity = 2 ** bits;
const mask = capacity - 1;
const shift = 32 - bits;
const table = new Float64Array(capacity);
let hasZero = false;

function slotOf(key) {
    return Math.imul(key, 0x9E3779B9) >>> shift;
}

function contains(key) {
    if (key === 0) return hasZero;
    let h = slotOf(key);
    while (table[h] !== 0) {
        if (table[h] === key) return true;
        h = (h + 1) & mask;
    }
    return false;
}

function insert(key) {
    if (key === 0) { hasZero = true; return; }
    let h = slotOf(key);
    while (table[h] !== 0) {
        if (table[h] === key) return;
        h = (h + 1) & mask;
    }
    table[h] = key;
}

let count = 0;
for (let i = 0; i < n; i++) {
    if (contains(target - arr[i])) count++;
    insert(arr[i]);
}
console.log(count);
