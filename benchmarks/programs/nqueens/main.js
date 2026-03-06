function solve(row, cols, diag1, diag2, n) {
    if (row === n) return 1;
    let count = 0;
    let available = ((1 << n) - 1) & ~(cols | diag1 | diag2);
    while (available) {
        const bit = available & (-available);
        available ^= bit;
        count += solve(row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1, n);
    }
    return count;
}
const n = parseInt(process.argv[2]);
console.log(solve(0, 0, 0, 0, n));
