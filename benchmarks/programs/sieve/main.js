const limit = parseInt(process.argv[2]);
const sieve = new Uint8Array(limit + 1);
let count = 0;
let h = 0;
for (let i = 2; i <= limit; i++) {
    h = (h * 31 + (sieve[i] ? 0 : 1)) & 268435455;
    if (!sieve[i]) {
        count++;
        for (let j = i * i; j <= limit; j += i)
            sieve[j] = 1;
    }
}
console.log(count + " " + h);
