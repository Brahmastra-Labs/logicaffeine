// Weight-free decode attribution: every launch of one GDN layer, one attention layer and the head,
// at the shipped decode shape (B=32), timed individually on random buffers. Kernels are the EMITTED
// ones — the program's main is renamed away and this harness supplies its own.
#define main qwen_emitted_main
#include QWEN38_EMITTED   /* the emitted program; its main is renamed away */
#undef main
#include <vector>
#include <string>
#include <algorithm>
#include <functional>
#include <map>
__global__ void fillf(float* p,long n,float a){ for(long i=blockIdx.x*(long)blockDim.x+threadIdx.x;i<n;i+=(long)gridDim.x*blockDim.x){ unsigned h=(unsigned)(i*2654435761u); p[i]=a*(((h>>8)&0xffff)/32768.f-1.f);} }
__global__ void fillh(__half* p,long n,float a){ for(long i=blockIdx.x*(long)blockDim.x+threadIdx.x;i<n;i+=(long)gridDim.x*blockDim.x){ unsigned h=(unsigned)(i*2246822519u); p[i]=__float2half(a*(1.f+((h>>8)&0xff)/256.f));} }
__global__ void fillc(signed char* p,long n){ for(long i=blockIdx.x*(long)blockDim.x+threadIdx.x;i<n;i+=(long)gridDim.x*blockDim.x){ unsigned h=(unsigned)(i*374761393u); p[i]=(signed char)((h>>13)%61-30);} }
template<class T> void AF(T*& p,long n,float a=0.5f){ cudaMalloc(&p,n*sizeof(T)); if(sizeof(T)==4) fillf<<<1024,256>>>((float*)p,n,a); else if(sizeof(T)==2) fillh<<<1024,256>>>((__half*)p,n,a); }
void AC(signed char*& p,long n){ cudaMalloc(&p,n); fillc<<<1024,256>>>(p,n); }
void AQ(signed char*& q,__half*& s,long n,long k){ AC(q,n*k); AF(s,n*(k/32),0.002f); }
struct K{ std::string cat, name; std::function<void(cudaStream_t)> f; };
#define CTX 640
int main(){
  const long R=(long)B*32;  // buffers sized for the widest pass, as the program does
  AF(x,R*E); AF(tmp,R*E); AF(x2n,R*E); AF(x_f32,R*E); AF(pbuf,R*PMAX); AF(cbuf,R*10240);
  AF(obuf,R*6144); AF(gnbuf,R*6144); AF(gbuf,R*96); AF(betab,R*48,0.3f); AF(decayb,R*48,0.3f);
  AF(attnb,(long)B*6144); AF(ffbuf,R*34816); AF(logits,(long)B*VOCAB); AF(skp,143360L*R,0.01f);
  AC(aqE,R*E); AC(aqEl,R*E); AC(aqQ,R*6144); AC(aqQl,R*6144); AC(ffQ,R*E); AC(ffQl,R*E); AC(hqQ,R*17408); AC(hqQl,R*17408);
  AF(asE,R*(E/32),0.01f); AF(asQ,R*(6144/32),0.01f); AF(ffS,R*(E/32),0.01f); AF(hqS,R*(17408/32),0.01f);
   std::vector<int> pos(B,CTX), zero(B,0);
  cudaMalloc(&d_pos,B*4); cudaMalloc(&d_zero,B*4); cudaMalloc(&d_tok,B*4);
  cudaMemcpy(d_pos,pos.data(),B*4,cudaMemcpyHostToDevice); cudaMemcpy(d_zero,zero.data(),B*4,cudaMemcpyHostToDevice);
  AQ(qkvz0_q,qkvz0_s,16384,5120); AQ(Wo0_q,Wo0_s,5120,6144); AQ(gu0_q,gu0_s,34816,5120); AQ(Wd0_q,Wd0_s,5120,17408);
  AQ(qgkv3_q,qgkv3_s,14336,5120); AQ(Wo3_q,Wo3_s,5120,6144); AQ(Wout_q,Wout_s,VOCAB,5120);
  AF(an0_f,E,1.f); AF(fn0_f,E,1.f); AF(an3_f,E,1.f); AF(fn3_f,E,1.f); AF(on_f,E,1.f);
  AF(ba0_f,491520,0.02f); AF(Wc0_f,40960,0.2f); AF(Dt0_f,48,0.1f); AF(Al0_f,48,0.1f); AF(Gn0_f,128,1.f);
  AF(Qn3_f,256,1.f); AF(Kn3_f,256,1.f); AF(cs0,(long)B*4*10240,0.1f);
  cudaMalloc(&st0,(size_t)B*6144*128*sizeof(bq_state_t)); cudaMemset(st0,0,(size_t)B*6144*128*sizeof(bq_state_t));
  cudaMalloc(&kc3,(size_t)B*MAXSEQ*1024*sizeof(bq_kv_t)); cudaMalloc(&vc3,(size_t)B*MAXSEQ*1024*sizeof(bq_kv_t));
  cudaMemset(kc3,0,(size_t)B*MAXSEQ*1024*sizeof(bq_kv_t)); cudaMemset(vc3,0,(size_t)B*MAXSEQ*1024*sizeof(bq_kv_t));
  cudaDeviceSynchronize();
  if(cudaGetLastError()!=cudaSuccess){ printf("alloc/fill failed\n"); return 1; }
  printf("B=%d  context=%d  MAXSEQ=%d\n",B,CTX,MAXSEQ);
  std::vector<K> gdn={
   {"norm","add_rmsnorm (entry)",[](cudaStream_t s){ bq_add_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,tmp,aqE,aqEl,asE,x_f32,an0_f,E,EPSV);}},
   {"gemm","qkvz 16384x5120 s8",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(128,8,((B)+TC_TJ-1)/TC_TJ),256,0,s>>>(pbuf,qkvz0_q,qkvz0_s,aqE,asE,5120,16384,B,16384,skp,8);}},
   {"reduce","sk_reduce qkvz",[](cudaStream_t s){ bq_sk_reduce<<<Gs(16384*(B)),256,0,s>>>(pbuf,skp,16384,B,16384,8);}},
   {"gdn","gemv_f32w (b-proj)",[](cudaStream_t s){ bq_gemv_f32w<<<dim3((96+(8/F32W_NW)-1)/(8/F32W_NW),B),256,0,s>>>(gbuf,ba0_f,x_f32,E,96,B);}},
   {"conv","conv_silu_l2",[](cudaStream_t s){ bq_conv_silu_l2<<<dim3(10240/128,B),128,0,s>>>(pbuf,PMAX,Wc0_f,cs0,cbuf,10240,4,32,EPSV);}},
   {"gdn","gdn_gates",[](cudaStream_t s){ bq_gdn_gates<<<dim3(Gs(48),B),256,0,s>>>(gbuf,Dt0_f,Al0_f,betab,decayb,48);}},
   {"gdn","gdn_chunk (state RMW)",[](cudaStream_t s){ bq_gdn_chunk<<<dim3(48,BQ_GDN_CHUNK_GY(128),B),128,0,s>>>(cbuf,cbuf+2048,cbuf+4096,betab,decayb,st0,obuf,128,16,48,10240,1,B);}},
   {"conv","gate_norm_hl",[](cudaStream_t s){ bq_gate_norm_hl<<<dim3(48,B),128,0,s>>>(pbuf+10240,Gn0_f,obuf,gnbuf,aqQ,aqQl,asQ,128,48,PMAX,EPSV);}},
   {"gemm","Wo 5120x6144",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(40,16,((B)+TC_TJ-1)/TC_TJ),256,0,s>>>(tmp,Wo0_q,Wo0_s,aqQ,asQ,6144,5120,B,5120,skp,16);}},
   {"reduce","sk_reduce Wo",[](cudaStream_t s){ bq_sk_reduce<<<Gs(5120*(B)),256,0,s>>>(tmp,skp,5120,B,5120,16);}},
   {"norm","add_rmsnorm (ffn)",[](cudaStream_t s){ bq_add_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,tmp,ffQ,ffQl,ffS,x2n,fn0_f,E,EPSV);}},
   {"gemm","gate/up 34816x5120",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(272,4,((B)+TC_TJ-1)/TC_TJ),256,0,s>>>(ffbuf,gu0_q,gu0_s,ffQ,ffS,5120,34816,B,34816,skp,4);}},
   {"silu","silu_had_sk (folds reduce)",[](cudaStream_t s){ bq_silu_had_q8_hl_sk<<<dim3(Gs(17408),B),256,0,s>>>(skp,4,hqQ,hqQl,hqS,17408);}},
   {"gemm","down 5120x17408",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(40,16,((B)+TC_TJ-1)/TC_TJ),256,0,s>>>(tmp,Wd0_q,Wd0_s,hqQ,hqS,17408,5120,B,5120,skp,16);}},
   {"reduce","sk_reduce down",[](cudaStream_t s){ bq_sk_reduce<<<Gs(5120*(B)),256,0,s>>>(tmp,skp,5120,B,5120,16);}},
  };
  std::vector<K> att={
   {"norm","add_rmsnorm (entry)",[](cudaStream_t s){ bq_add_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,tmp,aqE,aqEl,asE,x_f32,an3_f,E,EPSV);}},
   {"gemm","qgkv 14336x5120",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(112,10,((B)+TC_TJ-1)/TC_TJ),256,0,s>>>(pbuf,qgkv3_q,qgkv3_s,aqE,asE,5120,14336,B,16384,skp,10);}},
   {"reduce","sk_reduce qgkv",[](cudaStream_t s){ bq_sk_reduce<<<Gs(14336*(B)),256,0,s>>>(pbuf,skp,14336,B,16384,10);}},
   {"attn","kv_store",[](cudaStream_t s){ bq_kv_store<<<dim3(Gs(1024*2),B),256,0,s>>>(kc3,vc3,d_pos,1024,pbuf+12288,PMAX);}},
   {"attn","rope q",[](cudaStream_t s){ bq_qk_norm_rope_part<<<dim3(24,B),128,0,s>>>(pbuf,d_zero,0,PMAX,Qn3_f,24,256,64,ROPEV,EPSV);}},
   {"attn","rope k (cache)",[](cudaStream_t s){ bq_qk_norm_rope_kv<<<dim3(4,B),128,0,s>>>(kc3,d_pos,1024,MAXSEQ*1024,Kn3_f,4,256,64,ROPEV,EPSV);}},
   {"attn","attention (ctx 640) GQA",[](cudaStream_t s){ bq_attn_launch(s,attnb,pbuf,PMAX,kc3,vc3,d_pos,24,4,256,SCALEV,B);}},
   {"attn","gate_mul",[](cudaStream_t s){ bq_gate_mul_q8_hl<<<dim3(Gs(6144),B),256,0,s>>>(attnb,6144,pbuf+6144,PMAX,aqQ,aqQl,asQ,6144);}},
   {"gemm","Wo 5120x6144",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(40,16,((B)+TC_TJ-1)/TC_TJ),256,0,s>>>(tmp,Wo3_q,Wo3_s,aqQ,asQ,6144,5120,B,5120,skp,16);}},
   {"reduce","sk_reduce Wo",[](cudaStream_t s){ bq_sk_reduce<<<Gs(5120*(B)),256,0,s>>>(tmp,skp,5120,B,5120,16);}},
   {"norm","add_rmsnorm (ffn)",[](cudaStream_t s){ bq_add_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,tmp,ffQ,ffQl,ffS,x2n,fn3_f,E,EPSV);}},
   {"gemm","gate/up 34816x5120",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(272,4,((B)+TC_TJ-1)/TC_TJ),256,0,s>>>(ffbuf,gu0_q,gu0_s,ffQ,ffS,5120,34816,B,34816,skp,4);}},
   {"silu","silu_had_sk",[](cudaStream_t s){ bq_silu_had_q8_hl_sk<<<dim3(Gs(17408),B),256,0,s>>>(skp,4,hqQ,hqQl,hqS,17408);}},
   {"gemm","down 5120x17408",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(40,16,((B)+TC_TJ-1)/TC_TJ),256,0,s>>>(tmp,Wd0_q,Wd0_s,hqQ,hqS,17408,5120,B,5120,skp,16);}},
   {"reduce","sk_reduce down",[](cudaStream_t s){ bq_sk_reduce<<<Gs(5120*(B)),256,0,s>>>(tmp,skp,5120,B,5120,16);}},
  };
  std::vector<K> head={
   {"norm","final rmsnorm",[](cudaStream_t s){ bq_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,aqE,aqEl,asE,x_f32,on_f,E,EPSV);}},
   {"gemm","LM head 248320x5120",[](cudaStream_t s){ bq_gemm_tc1<<<dim3(1940,1),256,0,s>>>(logits,Wout_q,Wout_s,aqE,asE,E,VOCAB,B,VOCAB,(float*)0,1);}},
   {"head","argmax",[](cudaStream_t s){ std::vector<int> p(B,CTX); cudaMemcpyAsync(d_pos,p.data(),B*4,cudaMemcpyHostToDevice,s); bq_argmax_adv<<<B,256,0,s>>>(logits,VOCAB,d_tok,d_pos);}},
  };
  cudaStream_t s; cudaStreamCreate(&s); cudaEvent_t a,b; cudaEventCreate(&a); cudaEventCreate(&b);
  std::map<std::string,double> cat;
  auto timeit=[&](std::vector<K>& ks,int mult,const char* title){
    printf("\n== %s (x%d per step) ==\n",title,mult); double tot=0;
    for(auto& k:ks){ for(int i=0;i<3;i++) k.f(s); cudaStreamSynchronize(s);
      if(cudaGetLastError()!=cudaSuccess){ printf("  %-28s LAUNCH ERROR\n",k.name.c_str()); continue; }
      std::vector<float> v; for(int r=0;r<15;r++){ cudaEventRecord(a,s); k.f(s); cudaEventRecord(b,s); cudaEventSynchronize(b); float ms; cudaEventElapsedTime(&ms,a,b); v.push_back(ms);}
      std::sort(v.begin(),v.end()); double med=v[7]; tot+=med; cat[k.cat]+=med*mult;
      printf("  %-6s %-28s %8.4f ms\n",k.cat.c_str(),k.name.c_str(),med); }
    printf("  layer total %.4f ms  -> %.2f ms per step\n",tot,tot*mult); };
  timeit(gdn,48,"GDN layer"); timeit(att,16,"attention layer"); timeit(head,1,"head");
  double all=0; for(auto& kv:cat) all+=kv.second;
  printf("\n== one decode step, attributed (sum of isolated kernels) ==\n");
  std::vector<std::pair<double,std::string>> o; for(auto& kv:cat) o.push_back({kv.second,kv.first}); std::sort(o.rbegin(),o.rend());
  for(auto& p:o) printf("  %-7s %8.2f ms  %5.1f%%\n",p.second.c_str(),p.first,100*p.first/all);
  printf("  total   %8.2f ms   (%.0f tok/s at B=%d)\n",all,1000.0*B/all,B);
  return 0; }
