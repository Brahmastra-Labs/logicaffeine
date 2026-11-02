MOD = 1000000007
NREG = 8

n = ARGV[0].to_i
seed = 42
regs = Array.new(NREG) { |i| i + 1 }
n.times do
  seed = (seed * 1103515245 + 12345) % 2147483648
  instr = (seed >> 8) & 0x7fffff
  op = instr % 8
  dst = (instr / 8) % NREG
  a = (instr / 64) % NREG
  imm = (instr / 512) % 251
  case op
  when 0 then regs[dst] = imm
  when 1 then regs[dst] = (regs[dst] + regs[a]) % MOD
  when 2 then regs[dst] = (regs[dst] + MOD - regs[a]) % MOD
  when 3 then regs[dst] = (regs[dst] * regs[a] + 1) % MOD
  when 4 then regs[dst] = (regs[dst] + imm) % MOD
  when 5 then regs[dst] = (regs[dst] * (imm + 1)) % MOD
  when 6 then regs[dst] = regs[a]
  when 7 then regs[dst] = (regs[dst] + regs[a] + imm) % MOD
  end
end
checksum = 0
NREG.times do |i|
  checksum = (checksum * 31 + regs[i]) & 268435455
end
puts checksum
