const n = parseInt(process.argv[2]);
let sum = 0;
for (let i = 1; i <= n; i++) {
    sum = (sum + i) % 1000000007;
}
console.log(sum);
