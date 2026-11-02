const n = parseInt(process.argv[2]);
const arr = [];
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = (Math.imul(seed, 1103515245) + 12345) >>> 0;
    arr.push((seed >>> 16) & 0x7fff);
}
for (let i = 0; i < n - 1; i++)
    for (let j = 0; j < n - 1 - i; j++)
        if (arr[j] > arr[j + 1]) {
            const tmp = arr[j];
            arr[j] = arr[j + 1];
            arr[j + 1] = tmp;
        }
let checksum = 0;
for (const v of arr) checksum = (checksum * 31 + v) & 268435455;
console.log(`${arr[0]} ${arr[n - 1]} ${checksum}`);
