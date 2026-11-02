// collect — the table main.c hand-rolls: flat (key, value) slots, here interleaved in one Float64Array (doubles hold
// these keys and values exactly), a power-of-two capacity at least twice n, Fibonacci hashing (JavaScript has no
// 64-bit multiply, so the key times 2^32/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory
// and the key 0 itself kept beside the table.
const n = parseInt(process.argv[2]);
let bits = 4;
while (2 ** bits < 2 * n) bits++;
const capacity = 2 ** bits;
const mask = capacity - 1;
const shift = 32 - bits;
const table = new Float64Array(2 * capacity);
let hasZero = false, zeroValue = 0;

function slotOf(key) {
    return Math.imul(key, 0x9E3779B9) >>> shift;
}

function insert(key, value) {
    if (key === 0) { hasZero = true; zeroValue = value; return; }
    let i = slotOf(key);
    while (table[2 * i] !== 0 && table[2 * i] !== key) i = (i + 1) & mask;
    table[2 * i] = key;
    table[2 * i + 1] = value;
}

// The slot holding `key`, -1 when it holds none, -2 for the key 0.
function find(key) {
    if (key === 0) return hasZero ? -2 : -1;
    let i = slotOf(key);
    while (table[2 * i] !== 0) {
        if (table[2 * i] === key) return i;
        i = (i + 1) & mask;
    }
    return -1;
}

function valueAt(slot) {
    return slot === -2 ? zeroValue : table[2 * slot + 1];
}

let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    insert(seed, seed * 2);
}
let hit = 42, miss = seed, found = 0, h = 0;
for (let i = 0; i < n; i++) {
    hit = ((Math.imul(hit, 1103515245) + 12345) >>> 0) % 2147483648;
    let s = find(hit);
    if (s !== -1) { found++; h = (h * 31 + valueAt(s)) & 268435455; }
    miss = ((Math.imul(miss, 1103515245) + 12345) >>> 0) % 2147483648;
    s = find(miss);
    if (s !== -1) { found++; h = (h * 31 + valueAt(s)) & 268435455; }
}
console.log(`${found} ${h}`);
