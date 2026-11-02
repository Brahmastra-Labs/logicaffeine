const n = parseInt(process.argv[2]);
const parts = [];
for (let i = 0; i < n; i++)
    parts.push(String(i) + ' ');
const result = parts.join('');
let spaces = 0;
for (let i = 0; i < result.length; i++)
    if (result.charCodeAt(i) === 32) spaces++;
let h = 0;
for (let i = 0; i < result.length; i++)
    h = (h * 31 + result.charCodeAt(i)) & 268435455;
console.log(spaces + " " + h);
