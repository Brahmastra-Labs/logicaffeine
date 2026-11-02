# collect — the table main.c hand-rolls: flat (key, value) slots, here one array holding each key beside its value, a
# power-of-two capacity at least twice n, Fibonacci hashing (the key times 2^64/phi, its top 64-bit bits), linear
# probing, key 0 as the empty mark over a zeroed array and the key 0 itself kept beside the table.
PHI64 = 0x9E3779B97F4A7C15
MASK64 = (1 << 64) - 1

class Table
  def initialize(n)
    bits = 4
    bits += 1 while (1 << bits) < 2 * n
    @mask = (1 << bits) - 1
    @shift = 64 - bits
    @slots = Array.new(2 << bits, 0)
    @has_zero = false
    @zero_value = 0
  end

  def slot_of(key)
    ((key * PHI64) & MASK64) >> @shift
  end

  def insert(key, value)
    if key == 0
      @has_zero = true
      @zero_value = value
      return
    end
    i = slot_of(key)
    i = (i + 1) & @mask while @slots[2 * i] != 0 && @slots[2 * i] != key
    @slots[2 * i] = key
    @slots[2 * i + 1] = value
  end

  def find(key)
    return(@has_zero ? @zero_value : nil) if key == 0
    i = slot_of(key)
    while @slots[2 * i] != 0
      return @slots[2 * i + 1] if @slots[2 * i] == key
      i = (i + 1) & @mask
    end
    nil
  end
end

def main
  n = ARGV[0].to_i
  table = Table.new(n)
  seed = 42
  n.times do
    seed = (seed * 1103515245 + 12345) % 2147483648
    table.insert(seed, seed * 2)
  end
  hit = 42
  miss = seed
  found = 0
  h = 0
  n.times do
    hit = (hit * 1103515245 + 12345) % 2147483648
    v = table.find(hit)
    if v
      found += 1
      h = (h * 31 + v) & 268435455
    end
    miss = (miss * 1103515245 + 12345) % 2147483648
    v = table.find(miss)
    if v
      found += 1
      h = (h * 31 + v) & 268435455
    end
  end
  puts "#{found} #{h}"
end

main
