/* The entropy stream `randomInt`/`randomFloat` draw from: ChaCha20 (RFC 8439) keyed from the operating system, one
   stream per thread. A refill runs the block function four times under the current key, keeps the first 32 bytes
   of that output as the next key and hands out the other 224, erasing each word as it goes (fast key erasure): the
   generator's state never holds a word it has handed out, and no run of draws lets a later one be computed. A
   system that refuses entropy stops the program, since there is no weaker source worth falling back to. */
#include <sys/random.h>
#define LC_RNG_POOL 28
#ifdef __cplusplus
#define LC_RNG_THREAD thread_local
#else
#define LC_RNG_THREAD _Thread_local
#endif
static LC_RNG_THREAD __attribute__((unused)) uint32_t lc_rng_key[8];
static LC_RNG_THREAD __attribute__((unused)) uint64_t lc_rng_pool[LC_RNG_POOL];
static LC_RNG_THREAD __attribute__((unused)) int lc_rng_left, lc_rng_keyed;
__attribute__((unused)) static uint32_t lc_rng_rotl(uint32_t v, int n){ return (v << n) | (v >> (32 - n)); }
__attribute__((unused)) static void lc_rng_quarter(uint32_t *x, int a, int b, int c, int d){
    x[a]+=x[b]; x[d]=lc_rng_rotl(x[d]^x[a], 16);
    x[c]+=x[d]; x[b]=lc_rng_rotl(x[b]^x[c], 12);
    x[a]+=x[b]; x[d]=lc_rng_rotl(x[d]^x[a], 8);
    x[c]+=x[d]; x[b]=lc_rng_rotl(x[b]^x[c], 7);
}
__attribute__((unused)) static void lc_rng_refill(void){
    if(!lc_rng_keyed){
        if(getentropy(lc_rng_key, sizeof lc_rng_key)!=0) logos_fatal("randomInt(): the system gave no entropy to draw from");
        lc_rng_keyed=1;
    }
    uint32_t out[64];
    for(uint32_t block=0; block<4; block++){
        uint32_t s[16]={0x61707865u, 0x3320646eu, 0x79622d32u, 0x6b206574u,
            lc_rng_key[0], lc_rng_key[1], lc_rng_key[2], lc_rng_key[3],
            lc_rng_key[4], lc_rng_key[5], lc_rng_key[6], lc_rng_key[7], block, 0, 0, 0};
        uint32_t x[16];
        memcpy(x, s, sizeof x);
        for(int round=0; round<10; round++){
            lc_rng_quarter(x, 0, 4, 8, 12); lc_rng_quarter(x, 1, 5, 9, 13);
            lc_rng_quarter(x, 2, 6, 10, 14); lc_rng_quarter(x, 3, 7, 11, 15);
            lc_rng_quarter(x, 0, 5, 10, 15); lc_rng_quarter(x, 1, 6, 11, 12);
            lc_rng_quarter(x, 2, 7, 8, 13); lc_rng_quarter(x, 3, 4, 9, 14);
        }
        for(int i=0; i<16; i++) out[16*block+i]=x[i]+s[i];
    }
    memcpy(lc_rng_key, out, sizeof lc_rng_key);
    memcpy(lc_rng_pool, out+8, sizeof lc_rng_pool);
    for(volatile uint32_t *w=out; w<out+64; w++) *w=0;
    lc_rng_left=LC_RNG_POOL;
}
__attribute__((unused)) static uint64_t lc_rng_next(void){
    if(lc_rng_left==0) lc_rng_refill();
    uint64_t r=lc_rng_pool[--lc_rng_left];
    lc_rng_pool[lc_rng_left]=0;
    return r;
}
/* A uniform draw from `lo..=hi`: a word at or above the largest multiple of the span is drawn again, so no value is
   likelier than another. */
__attribute__((unused)) static int64_t lc_random_int(int64_t lo, int64_t hi){
    if(lo > hi) logos_fatalf(@RANDOM_BOUNDS@, lo, hi);
    uint64_t span=(uint64_t)hi-(uint64_t)lo+1u;
    if(span==0) return (int64_t)lc_rng_next();
    uint64_t tail=(UINT64_MAX % span + 1u) % span, r;
    do { r=lc_rng_next(); } while(r > UINT64_MAX - tail);
    return (int64_t)((uint64_t)lo + r % span);
}
__attribute__((unused)) static double lc_random_float(void){ return (double)(lc_rng_next() >> 11) / (double)(1ULL << 53); }
