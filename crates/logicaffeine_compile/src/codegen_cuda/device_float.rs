//! The device override for `logos_fmt_f64` — a from-scratch shortest-round-trip `f64` formatter for
//! CUDA device code, byte-identical to the host `logos_fmt_f64` (Rust `{}` Display: the shortest
//! decimal that round-trips, NEVER scientific notation, bare integral floats, `NaN`/`inf`/`-inf`).
//!
//! The device has no `strtod`/`snprintf`, so the host's round-trip search cannot be lifted. Instead
//! this is Steele & White's **Dragon4** free-format algorithm (`FPP2`) over a small fixed-limb
//! big-integer: exact, table-free, and — using repeated subtraction for each digit (a base-10 digit
//! needs ≤ 9 subtractions once `R < S`) — needing no big division. The emitted decimal mantissa +
//! point position feed the SAME never-scientific reformatting tail the host uses, so every finite
//! value prints identically. Proven ≡ the host formatter by a GPU fuzz (`cuda_float_lock`).

/// The device `__device__` C source for `logos_fmt_f64` (and its bignum helpers). Substituted for the
/// host body when the CUDA device-lift reaches `logos_fmt_f64` (see `super::device_lift`). Self-
/// contained: references only `malloc`/`memcpy`/`isnan`/`isinf` (all device-available).
pub const LOGOS_DEVICE_FLOAT_FMT: &str = r#"
// ---- device shortest-round-trip f64 formatter (Dragon4 / FPP2) --------------------------------
// A fixed-limb little-endian base-2^32 big integer. 96 limbs = 3072 bits covers 2^1076 (the widest
// denominator for a subnormal double) times the ~10^340 decimal scaling, with headroom.
typedef struct { unsigned int d[96]; int n; } LFBig;
__device__ static void lfb_norm(LFBig* a){ while(a->n>0 && a->d[a->n-1]==0) a->n--; }
__device__ static void lfb_from_u64(LFBig* a, unsigned long long v){ a->d[0]=(unsigned int)v; a->d[1]=(unsigned int)(v>>32); a->n=2; lfb_norm(a); }
__device__ static void lfb_copy(LFBig* a, const LFBig* b){ a->n=b->n; for(int i=0;i<b->n;i++) a->d[i]=b->d[i]; }
__device__ static void lfb_shl(LFBig* a, int bits){
    if(a->n==0 || bits<=0) return;
    int words=bits>>5, rem=bits&31;
    if(words){ for(int i=a->n-1;i>=0;i--) a->d[i+words]=a->d[i]; for(int i=0;i<words;i++) a->d[i]=0; a->n+=words; }
    if(rem){ unsigned int carry=0; for(int i=words;i<a->n;i++){ unsigned long long x=((unsigned long long)a->d[i]<<rem)|carry; a->d[i]=(unsigned int)x; carry=(unsigned int)(x>>32);} if(carry && a->n<96) a->d[a->n++]=carry; }
}
__device__ static void lfb_mul10(LFBig* a){
    unsigned long long carry=0; for(int i=0;i<a->n;i++){ unsigned long long x=(unsigned long long)a->d[i]*10ull+carry; a->d[i]=(unsigned int)x; carry=x>>32; } if(carry && a->n<96) a->d[a->n++]=(unsigned int)carry;
}
__device__ static int lfb_cmp(const LFBig* a, const LFBig* b){
    if(a->n!=b->n) return a->n<b->n?-1:1;
    for(int i=a->n-1;i>=0;i--) if(a->d[i]!=b->d[i]) return a->d[i]<b->d[i]?-1:1;
    return 0;
}
__device__ static void lfb_add(LFBig* r, const LFBig* a, const LFBig* b){ // r = a + b (r may alias a)
    int n = a->n>b->n?a->n:b->n; unsigned long long carry=0;
    for(int i=0;i<n;i++){ unsigned long long x=carry+(i<a->n?a->d[i]:0)+(i<b->n?b->d[i]:0); r->d[i]=(unsigned int)x; carry=x>>32; }
    r->n=n; if(carry && r->n<96) r->d[r->n++]=(unsigned int)carry;
}
__device__ static void lfb_sub(LFBig* a, const LFBig* b){ // a -= b, requires a >= b
    long long borrow=0; for(int i=0;i<a->n;i++){ long long x=(long long)a->d[i]-(i<b->n?(long long)b->d[i]:0)-borrow; if(x<0){ x+=(1ll<<32); borrow=1; } else borrow=0; a->d[i]=(unsigned int)x; } lfb_norm(a);
}

__device__ static char* logos_fmt_f64(double x){
    if(isnan(x)){ char* r=(char*)malloc(4); r[0]='N'; r[1]='a'; r[2]='N'; r[3]=0; return r; }
    if(isinf(x)){ char* r=(char*)malloc(5); int o=0; if(x<0) r[o++]='-'; r[o++]='i'; r[o++]='n'; r[o++]='f'; r[o]=0; return r; }
    unsigned long long bits; memcpy(&bits, &x, 8);
    int sign = (int)(bits>>63);
    int ieeeExp = (int)((bits>>52) & 0x7ff);
    unsigned long long ieeeMant = bits & 0xfffffffffffffull;
    if(ieeeExp==0 && ieeeMant==0){ char* r=(char*)malloc(3); int o=0; if(sign) r[o++]='-'; r[o++]='0'; r[o]=0; return r; }
    unsigned long long m2; int e2;
    if(ieeeExp==0){ m2=ieeeMant; e2=1-1023-52; } else { m2=ieeeMant | (1ull<<52); e2=ieeeExp-1023-52; }
    int unequalGaps = (ieeeMant==0 && ieeeExp>1);

    // v = m2 * 2^e2. Set up R/S/M+ (Mp)/M- (Mm) so that v = R/S and the ulp gaps are Mp/S, Mm/S.
    LFBig R,S,Mp,Mm,tmp;
    if(e2>=0){
        LFBig be; lfb_from_u64(&be,1); lfb_shl(&be,e2);
        lfb_from_u64(&R,m2); lfb_shl(&R,e2);
        if(!unequalGaps){ lfb_shl(&R,1); lfb_from_u64(&S,2); lfb_copy(&Mp,&be); lfb_copy(&Mm,&be); }
        else { lfb_shl(&R,2); lfb_from_u64(&S,4); lfb_copy(&Mp,&be); lfb_shl(&Mp,1); lfb_copy(&Mm,&be); }
    } else {
        lfb_from_u64(&R,m2);
        if(!unequalGaps){ lfb_shl(&R,1); lfb_from_u64(&S,1); lfb_shl(&S,-e2); lfb_shl(&S,1); lfb_from_u64(&Mp,1); lfb_from_u64(&Mm,1); }
        else { lfb_shl(&R,2); lfb_from_u64(&S,1); lfb_shl(&S,-e2); lfb_shl(&S,2); lfb_from_u64(&Mp,2); lfb_from_u64(&Mm,1); }
    }

    // Scale by a power of ten so that the first emitted digit is the leading significant digit; k is
    // the count of digits to the LEFT of the decimal point (pointpos). Two mutually-exclusive fixups.
    int k = 0;
    // while R + Mp > S : S *= 10 ; k++
    for(;;){ lfb_add(&tmp,&R,&Mp); if(lfb_cmp(&tmp,&S)>0){ lfb_mul10(&S); k++; } else break; if(k>400) break; }
    // while (R + Mp) * 10 <= S : R,Mp,Mm *= 10 ; k--
    for(;;){ lfb_add(&tmp,&R,&Mp); lfb_mul10(&tmp); if(lfb_cmp(&tmp,&S)<=0){ lfb_mul10(&R); lfb_mul10(&Mp); lfb_mul10(&Mm); k--; } else break; if(k<-400) break; }

    char digits[40]; int nd=0;
    for(int guard=0; guard<40; guard++){
        lfb_mul10(&R); lfb_mul10(&Mp); lfb_mul10(&Mm);
        int d=0; while(lfb_cmp(&R,&S)>=0){ lfb_sub(&R,&S); d++; }
        int low = lfb_cmp(&R,&Mm) < 0;
        lfb_add(&tmp,&R,&Mp);
        int high = lfb_cmp(&tmp,&S) > 0;
        if(low || high){
            // final digit: round per the two boundary flags (halfway → compare 2R vs S).
            if(low && !high) { /* keep d */ }
            else if(high && !low) { d++; }
            else { LFBig r2; lfb_add(&r2,&R,&R); if(lfb_cmp(&r2,&S) >= 0) d++; }
            digits[nd++]=(char)('0'+d);
            break;
        }
        digits[nd++]=(char)('0'+d);
    }

    int pointpos = k;
    char* out=(char*)malloc(768); int oi=0;
    if(sign) out[oi++]='-';
    if(pointpos<=0){
        out[oi++]='0'; out[oi++]='.';
        for(int i=0;i<-pointpos;i++) out[oi++]='0';
        for(int i=0;i<nd;i++) out[oi++]=digits[i];
    } else if(pointpos>=nd){
        for(int i=0;i<nd;i++) out[oi++]=digits[i];
        for(int i=0;i<pointpos-nd;i++) out[oi++]='0';
    } else {
        for(int i=0;i<pointpos;i++) out[oi++]=digits[i];
        out[oi++]='.';
        for(int i=pointpos;i<nd;i++) out[oi++]=digits[i];
    }
    out[oi]=0;
    return out;
}
"#;
