const n = parseInt(process.argv[2]);
const map = new Map();
for (let i = 0; i < n; i++)
    map.set(i, i * 2);
let found = 0;
for (let i = 0; i < n; i++)
    if (map.get(i) === i * 2) found++;
console.log(found);
