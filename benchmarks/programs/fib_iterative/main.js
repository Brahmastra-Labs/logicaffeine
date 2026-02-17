const n = parseInt(process.argv[2]);
let a = 0, b = 1;
for (let i = 0; i < n; i++) {
    const temp = b;
    b = (a + b) % 1000000007;
    a = temp;
}
console.log(a);
