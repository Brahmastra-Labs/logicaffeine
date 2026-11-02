# two_sum — the set main.c hand-rolls: flat key slots, a power-of-two capacity at least twice n, Fibonacci hashing (the
# key times 2^64/phi, its top 64-bit bits), linear probing, key 0 as the empty mark over a zeroed array and the key 0
# itself, which a complement can be, a flag beside the table.
PHI64 = 0x9E3779B97F4A7C15
MASK64 = (1 << 64) - 1

class KeySet
  def initialize(n)
    bits = 4
    bits += 1 while (1 << bits) < 2 * n
    @mask = (1 << bits) - 1
    @shift = 64 - bits
    @slots = Array.new(1 << bits, 0)
    @has_zero = false
  end

  def slot_of(key)
    ((key * PHI64) & MASK64) >> @shift
  end

  def contains?(key)
    return @has_zero if key == 0
    h = slot_of(key)
    while @slots[h] != 0
      return true if @slots[h] == key
      h = (h + 1) & @mask
    end
    false
  end

  def insert(key)
    if key == 0
      @has_zero = true
      return
    end
    h = slot_of(key)
    while @slots[h] != 0
      return if @slots[h] == key
      h = (h + 1) & @mask
    end
    @slots[h] = key
  end
end

n = ARGV[0].to_i
target = 2147483647
arr = Array.new(n, 0); seed = 42
n.times { |i| seed=(seed*1103515245+12345)%2147483648; arr[i] = (i > 0 && (seed >> 28) == 0 ? target - arr[seed % i] : seed) }
seen = KeySet.new(n); count = 0
arr.each { |x| count += 1 if seen.contains?(target - x); seen.insert(x) }
puts count
