n = ARGV[0].to_i
count = 0
n.times do |y|
  n.times do |x|
    cr = 2.0*x/n - 1.5; ci = 2.0*y/n - 1.0; zr = zi = 0.0; inside = true
    50.times do
      zr, zi = zr*zr - zi*zi + cr, 2*zr*zi + ci
      if zr*zr + zi*zi > 4; inside = false; break; end
    end
    count += 1 if inside
  end
end
puts count
