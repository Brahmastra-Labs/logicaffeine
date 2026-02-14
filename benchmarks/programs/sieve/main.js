const limit = parseInt(process.argv[2]);
const sieve = new Uint8Array(limit + 1);
let count = 0;
for (let i = 2; i <= limit; i++) {
    if (!sieve[i]) {
        count++;
        for (let j = i * i; j <= limit; j += i)
            sieve[j] = 1;
    }
}
console.log(count);
