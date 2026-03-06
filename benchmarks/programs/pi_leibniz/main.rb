n = ARGV[0].to_i
sum = 0.0; sign = 1.0
n.times { |k| sum += sign / (2.0 * k + 1.0); sign = -sign }
printf("%.15f
", sum * 4.0)
