import sys

MOD = 1000000007
NREG = 8


def main():
    n = int(sys.argv[1])
    seed = 42
    regs = [i + 1 for i in range(NREG)]
    for _ in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        instr = (seed >> 8) & 0x7fffff
        op = instr % 8
        dst = (instr // 8) % NREG
        a = (instr // 64) % NREG
        imm = (instr // 512) % 251
        if op == 0:
            regs[dst] = imm
        elif op == 1:
            regs[dst] = (regs[dst] + regs[a]) % MOD
        elif op == 2:
            regs[dst] = (regs[dst] + MOD - regs[a]) % MOD
        elif op == 3:
            regs[dst] = (regs[dst] * regs[a] + 1) % MOD
        elif op == 4:
            regs[dst] = (regs[dst] + imm) % MOD
        elif op == 5:
            regs[dst] = (regs[dst] * (imm + 1)) % MOD
        elif op == 6:
            regs[dst] = regs[a]
        else:
            regs[dst] = (regs[dst] + regs[a] + imm) % MOD
    checksum = 0
    for i in range(NREG):
        checksum = (checksum * 31 + regs[i]) & 268435455
    print(checksum)


if __name__ == "__main__":
    main()
