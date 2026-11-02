function A(i,j){return 1/((i+j)*(i+j+1)/2+i+1)}
function mulAv(n,v,r){for(let i=0;i<n;i++){let s=0;for(let j=0;j<n;j++)s+=A(i,j)*v[j];r[i]=s}}
function mulAtv(n,v,r){for(let i=0;i<n;i++){let s=0;for(let j=0;j<n;j++)s+=A(j,i)*v[j];r[i]=s}}
function mulAtAv(n,v,r,t){mulAv(n,v,t);mulAtv(n,t,r)}
const n=parseInt(process.argv[2]);
const u=new Float64Array(n).fill(1),v=new Float64Array(n),t=new Float64Array(n);
for(let i=0;i<10;i++){mulAtAv(n,u,v,t);mulAtAv(n,v,u,t)}
let vBv=0,vv=0;
for(let i=0;i<n;i++){vBv+=u[i]*v[i];vv+=v[i]*v[i]}
console.log(Math.sqrt(vBv/vv).toFixed(9));
