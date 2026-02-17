function gcd(a, b) {
    while (b > 0) { const t = b; b = a % b; a = t; }
    return a;
}
const n = parseInt(process.argv[2]);
let sum = 0;
for (let i = 1; i <= n; i++)
    for (let j = i; j <= n; j++)
        sum += gcd(i, j);
console.log(sum);
