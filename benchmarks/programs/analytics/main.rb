# analytics — filter → map → group-by → reduce over N LCG-generated records.
# Reads N from ARGV[0]; prints two lines (kept count, checksum), byte-identical
# across every language. Ruby integers are arbitrary-precision, so the arithmetic
# stays exact with plain %, >>, and *.

MOD = 1000000007
NGROUP = 32768

def main
  n = ARGV[0].to_i
  seed = 42
  groups = Array.new(NGROUP, 0)
  kept = 0
  n.times do
    seed = (seed * 1103515245 + 12345) % 2147483648
    key = (seed >> 16) & 0x7fff
    seed = (seed * 1103515245 + 12345) % 2147483648
    value = (seed >> 16) & 0x7fff
    next if value % 3 == 0

    mapped = (value * key + 7) % MOD
    g = key % NGROUP
    groups[g] = (groups[g] + mapped) % MOD
    kept += 1
  end
  checksum = 0
  NGROUP.times do |g|
    checksum = (checksum * 31 + groups[g]) & 268435455
  end
  print "#{kept}\n#{checksum}\n"
end

main
