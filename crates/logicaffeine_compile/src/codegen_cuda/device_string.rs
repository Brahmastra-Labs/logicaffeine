//! Device string runtime — a from-scratch `vsnprintf`/`snprintf`/`sprintf` for CUDA device code, so the
//! C runtime's string builders (`logos_dyn_sprintf`, `i64_to_str`, the `*_to_string` renderers, and
//! interpolation via `apply_format_spec`) lift verbatim. The device has no `vsnprintf`; the lifted code
//! calls `snprintf`/`vsnprintf`/`sprintf`, which [`super::device_lift`] routes to these `logos_*`.
//!
//! Scope: the conversions the runtime actually emits — `%%`, `%c`, `%s` (with width/precision/`-`),
//! signed `%d`/`%i`/`%lld`/`%ld` and unsigned `%u`/`%llu`/`%lu`/`%x`/`%X` (with `0`/`-`/`+`/space flags,
//! width, `*`, precision, and `l`/`ll`/`z` length modifiers). Float conversions do NOT appear in the
//! string slice — `logos_fmt_f64` is a device OVERRIDE (Dragon4) that yields a string rendered via `%s`.
//! C99 return semantics: the length that WOULD be written, so `logos_dyn_sprintf`'s `(NULL,0)` sizing
//! pass works. Every rendering is proven ≡ the VM byte-for-byte by the Phase-3 corpus.

/// The `__device__` C source for `logos_vsnprintf`/`logos_snprintf`/`logos_sprintf`. Prepended (after the
/// allocator shim) whenever a program's slice builds strings. Self-contained: needs only `<cstdarg>`.
pub const CUDA_DEVICE_STRING_SHIM: &str = r#"// --- device string runtime (str* + vsnprintf/snprintf/sprintf) ------------------
#include <cstdarg>
// nvcc provides device memcpy/memset but NOT strlen/strcmp/strcpy/…; supply them.
__device__ static size_t logos_strlen(const char* s){ size_t n=0; while(s[n]) n++; return n; }
__device__ static int logos_strcmp(const char* a, const char* b){ while(*a && *a==*b){a++;b++;} return (int)(unsigned char)*a - (int)(unsigned char)*b; }
__device__ static int logos_strncmp(const char* a, const char* b, size_t n){ for(size_t i=0;i<n;i++){ if(a[i]!=b[i] || !a[i]) return (int)(unsigned char)a[i]-(int)(unsigned char)b[i]; } return 0; }
__device__ static char* logos_strcpy(char* d, const char* s){ char* r=d; while((*d++=*s++)){} return r; }
__device__ static char* logos_strncpy(char* d, const char* s, size_t n){ size_t i=0; for(; i<n && s[i]; i++) d[i]=s[i]; for(; i<n; i++) d[i]=0; return d; }
__device__ static char* logos_strcat(char* d, const char* s){ char* r=d; while(*d) d++; while((*d++=*s++)){} return r; }
__device__ static int logos_memcmp(const void* a, const void* b, size_t n){ const unsigned char* x=(const unsigned char*)a; const unsigned char* y=(const unsigned char*)b; for(size_t i=0;i<n;i++){ if(x[i]!=y[i]) return (int)x[i]-(int)y[i]; } return 0; }
__device__ static int logos_vsnprintf(char* out, size_t cap, const char* fmt, va_list ap){
    size_t pos = 0; // total chars that WOULD be written (C99 return)
    #define LOGOS_PUT(ch) do { char _c=(char)(ch); if(out && pos+1 < cap) out[pos]=_c; pos++; } while(0)
    for(const char* p=fmt; *p; ){
        if(*p != '%'){ LOGOS_PUT(*p); p++; continue; }
        p++;
        if(*p=='%'){ LOGOS_PUT('%'); p++; continue; }
        int left=0, zero=0, plus=0, space=0;
        for(;;){ if(*p=='-')left=1; else if(*p=='0')zero=1; else if(*p=='+')plus=1; else if(*p==' ')space=1; else break; p++; }
        int width=0; if(*p=='*'){ width=va_arg(ap,int); p++; if(width<0){ left=1; width=-width; } }
        else while(*p>='0'&&*p<='9'){ width=width*10+(*p-'0'); p++; }
        int prec=-1; if(*p=='.'){ p++; prec=0; if(*p=='*'){ prec=va_arg(ap,int); p++; } else while(*p>='0'&&*p<='9'){ prec=prec*10+(*p-'0'); p++; } }
        int lng=0; for(;;){ if(*p=='l'){ lng++; p++; } else if(*p=='h'){ p++; } else if(*p=='z'||*p=='j'||*p=='t'){ lng=2; p++; } else if(*p=='L'){ p++; } else break; }
        char conv=*p; if(conv) p++;
        if(conv=='s'){
            const char* s=va_arg(ap,const char*); if(!s) s="(null)";
            int slen=0; while(s[slen]) slen++;
            if(prec>=0 && prec<slen) slen=prec;
            int pad = width>slen? width-slen : 0;
            if(!left) for(int i=0;i<pad;i++) LOGOS_PUT(' ');
            for(int i=0;i<slen;i++) LOGOS_PUT(s[i]);
            if(left) for(int i=0;i<pad;i++) LOGOS_PUT(' ');
            continue;
        }
        if(conv=='c'){
            char c=(char)va_arg(ap,int);
            int pad = width>1? width-1:0;
            if(!left) for(int i=0;i<pad;i++) LOGOS_PUT(' ');
            LOGOS_PUT(c);
            if(left) for(int i=0;i<pad;i++) LOGOS_PUT(' ');
            continue;
        }
        // integer conversions
        int base=10; int upper=0; int is_signed = (conv=='d'||conv=='i');
        if(conv=='x'){ base=16; } else if(conv=='X'){ base=16; upper=1; } else if(conv=='u'){ base=10; }
        else if(!is_signed){ /* unknown conv — emit literally to be safe */ LOGOS_PUT('%'); LOGOS_PUT(conv); continue; }
        unsigned long long uv; int neg=0;
        if(is_signed){
            long long v = (lng>=2)? va_arg(ap,long long) : (lng==1)? (long long)va_arg(ap,long) : (long long)va_arg(ap,int);
            if(v<0){ neg=1; uv=(unsigned long long)(-(v+1))+1ull; } else uv=(unsigned long long)v;
        } else {
            uv = (lng>=2)? va_arg(ap,unsigned long long) : (lng==1)? (unsigned long long)va_arg(ap,unsigned long) : (unsigned long long)va_arg(ap,unsigned int);
        }
        char rev[32]; int rl=0;
        if(uv==0) rev[rl++]='0';
        while(uv){ int d=(int)(uv%base); rev[rl++]= d<10? ('0'+d) : ((upper?'A':'a')+d-10); uv/=(unsigned long long)base; }
        // apply precision (min digits) with leading zeros
        while(prec>=0 && rl<prec) rev[rl++]='0';
        char sign = neg?'-': (is_signed&&plus)?'+': (is_signed&&space)?' ':0;
        int digits = rl + (sign?1:0);
        int pad = width>digits? width-digits : 0;
        if(zero && !left && prec<0){
            if(sign) LOGOS_PUT(sign);
            for(int i=0;i<pad;i++) LOGOS_PUT('0');
            for(int i=rl-1;i>=0;i--) LOGOS_PUT(rev[i]);
        } else if(!left){
            for(int i=0;i<pad;i++) LOGOS_PUT(' ');
            if(sign) LOGOS_PUT(sign);
            for(int i=rl-1;i>=0;i--) LOGOS_PUT(rev[i]);
        } else {
            if(sign) LOGOS_PUT(sign);
            for(int i=rl-1;i>=0;i--) LOGOS_PUT(rev[i]);
            for(int i=0;i<pad;i++) LOGOS_PUT(' ');
        }
    }
    if(out && cap>0) out[pos<cap?pos:cap-1]=0;
    #undef LOGOS_PUT
    return (int)pos;
}
__device__ static int logos_snprintf(char* out, size_t cap, const char* fmt, ...){
    va_list ap; va_start(ap, fmt); int r=logos_vsnprintf(out, cap, fmt, ap); va_end(ap); return r;
}
__device__ static int logos_sprintf(char* out, const char* fmt, ...){
    va_list ap; va_start(ap, fmt); int r=logos_vsnprintf(out, (size_t)-1, fmt, ap); va_end(ap); return r;
}
// --- lifted LOGOS runtime slice (device) ----------------------------------------
"#;
