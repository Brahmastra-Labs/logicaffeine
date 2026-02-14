function fib(n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

const n = parseInt(process.argv[2]);
console.log(fib(n));
