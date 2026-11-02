Node = Struct.new(:left, :right)
def bottom_up_tree(d); d>0 ? Node.new(bottom_up_tree(d-1), bottom_up_tree(d-1)) : Node.new(nil, nil); end
def check(node); node.left.nil? ? 1 : 1+check(node.left)+check(node.right); end
n=ARGV[0].to_i; mn=4; mx=[mn+2,n].max
puts "stretch tree of depth #{mx+1}\t check: #{check(bottom_up_tree(mx+1))}"
ll=bottom_up_tree(mx)
(mn..mx).step(2) do |d|
  it=1<<(mx-d+mn); tc=0; it.times{tc+=check(bottom_up_tree(d))}
  puts "#{it}\t trees of depth #{d}\t check: #{tc}"
end
puts "long lived tree of depth #{mx}\t check: #{check(ll)}"
