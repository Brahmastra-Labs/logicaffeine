const n = parseInt(process.argv[2]);
let text = [];
let pos = 0;
while (pos < n) {
    if (pos > 0 && pos % 1000 === 0 && pos + 5 <= n) {
        text.push('X','X','X','X','X');
        pos += 5;
    } else {
        text.push(String.fromCharCode(97 + pos % 5));
        pos++;
    }
}
const needle = "XXXXX";
let count = 0;
for (let i = 0; i <= text.length - 5; i++) {
    if (text[i]==='X' && text[i+1]==='X' && text[i+2]==='X' && text[i+3]==='X' && text[i+4]==='X') count++;
}
console.log(count);
