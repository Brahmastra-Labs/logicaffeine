const n = parseInt(process.argv[2]);
const parts = [];
for (let i = 0; i < n; i++)
    parts.push(String(i) + ' ');
const result = parts.join('');
let spaces = 0;
for (let i = 0; i < result.length; i++)
    if (result[i] === ' ') spaces++;
console.log(spaces);
