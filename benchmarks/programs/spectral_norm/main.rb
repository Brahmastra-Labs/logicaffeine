def a(i,j); 1.0/((i+j)*(i+j+1)/2+i+1); end
n=ARGV[0].to_i
u=Array.new(n,1.0)
v=Array.new(n,0.0)
10.times do
  t=Array.new(n){|i| (0...n).sum{|j| a(i,j)*u[j]}}
  v=Array.new(n){|i| (0...n).sum{|j| a(j,i)*t[j]}}
  t=Array.new(n){|i| (0...n).sum{|j| a(i,j)*v[j]}}
  u=Array.new(n){|i| (0...n).sum{|j| a(j,i)*t[j]}}
end
vBv=vv=0.0
n.times{|i| vBv+=u[i]*v[i]; vv+=v[i]*v[i]}
printf("%.9f\n", Math.sqrt(vBv/vv))
