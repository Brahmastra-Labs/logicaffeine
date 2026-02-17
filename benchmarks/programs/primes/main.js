const n = parseInt(process.argv[2]);
let count = 0;
for (let i = 2; i <= n; i++) {
    let isPrime = true;
    for (let d = 2; d * d <= i; d++) {
        if (i % d === 0) { isPrime = false; break; }
    }
    if (isPrime) count++;
}
console.log(count);
