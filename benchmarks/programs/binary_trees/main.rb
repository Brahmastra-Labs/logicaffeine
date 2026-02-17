def make_check(d); d==0 ? 1 : 1+make_check(d-1)+make_check(d-1); end
n=ARGV[0].to_i; mn=4; mx=[mn+2,n].max
puts "stretch tree of depth #{mx+1}\t check: #{make_check(mx+1)}"
ll=make_check(mx)
(mn..mx).step(2) do |d|
  it=1<<(mx-d+mn); tc=0; it.times{tc+=make_check(d)}
  puts "#{it}\t trees of depth #{d}\t check: #{tc}"
end
puts "long lived tree of depth #{mx}\t check: #{ll}"
