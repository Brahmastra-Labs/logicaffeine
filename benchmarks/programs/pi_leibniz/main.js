const n = parseInt(process.argv[2]);
let sum = 0, sign = 1;
for (let k = 0; k < n; k++) { sum += sign / (2 * k + 1); sign = -sign; }
console.log((sum * 4).toFixed(15));
