def a(i,j); 1.0/((i+j)*(i+j+1)/2+i+1); end

def mul_av(n, v, out)
  n.times do |i|
    out[i] = 0.0
    n.times { |j| out[i] += a(i,j) * v[j] }
  end
end

def mul_atv(n, v, out)
  n.times do |i|
    out[i] = 0.0
    n.times { |j| out[i] += a(j,i) * v[j] }
  end
end

n=ARGV[0].to_i
u=Array.new(n,1.0)
v=Array.new(n,0.0)
t=Array.new(n,0.0)
10.times do
  mul_av(n, u, t)
  mul_atv(n, t, v)
  mul_av(n, v, t)
  mul_atv(n, t, u)
end
vBv=vv=0.0
n.times{|i| vBv+=u[i]*v[i]; vv+=v[i]*v[i]}
printf("%.9f\n", Math.sqrt(vBv/vv))
