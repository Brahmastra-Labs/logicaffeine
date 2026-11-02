const n = parseInt(process.argv[2]);
const coins = [1, 5, 10, 25, 50, 100];
const dp = new Array(n + 1).fill(0);
dp[0] = 1;
for (const c of coins) for (let j = c; j <= n; j++) dp[j] = (dp[j] + dp[j - c]) % 1000000007;
let checksum = 0;
for (const v of dp) checksum = (checksum * 31 + v) & 268435455;
console.log(`${dp[n]} ${checksum}`);
