/* hft_baseline — the SAME tick-to-trade pipeline as hft_model.py / the LOGOS silicon program, in tight
 * -O2 C, timed per message on the host x86 with the cycle counter (rdtscp). This is the fair "fast CPU"
 * baseline: not to show we out-run it on median latency (a 6.25 MHz FPGA does not), but to show its
 * JITTER — the p99.9 tail from cache misses / branch mispredicts / OS preemption that our deterministic
 * silicon does not have. Prints a JSON line of the per-message latency distribution in nanoseconds.
 *
 *   cc -O2 -o /tmp/hft_baseline scripts/hft_baseline.c
 *   /tmp/hft_baseline <iters>          # replay the demo feed <iters> times, one sample per message
 *
 * The order stream it produces is verified byte-identical to hft_model.run_pipeline by the harness.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <math.h>
#include <x86intrin.h>

#define LEVELS 16
#define PRICE_MIN 93
#define FAIR_100 10050
#define EDGE_100 5
#define BASE_SIZE 10
#define MAX_ORDER 50
#define POS_CAP 100
#define TYPE_ADD 65

/* the demo feed (must equal hft_model.DEMO_FEED) — 8 messages x 8 bytes */
static const int DEMO_FEED[] = {
    65,1,0,100,0,10,0,1, 65,2,0,101,0,10,0,2, 65,1,0,100,0,30,0,3, 65,2,0,101,0,60,0,4,
    65,1,0,100,0,100,0,5, 69,2,0,101,0,50,0,4, 68,1,0,100,0,100,0,5, 65,2,0,101,0,200,0,7};
#define FEED_BYTES ((int)(sizeof(DEMO_FEED)/sizeof(DEMO_FEED[0])))

static long bid[LEVELS + 1], ask[LEVELS + 1], inventory;

/* process ONE message at feed[off..]; append its 3 Show ints to out (if non-NULL). Returns nothing. */
static inline void process_msg(const int *feed, int off, long *out, int *outn) {
    int mtype = feed[off], side = feed[off + 1];
    long price = feed[off + 2] * 256 + feed[off + 3];
    long qty = feed[off + 4] * 256 + feed[off + 5];
    long tick = price - (PRICE_MIN - 1);
    if (tick < 1) tick = 1;
    if (tick > LEVELS) tick = LEVELS;
    long signed_q = (mtype == TYPE_ADD) ? qty : -qty;
    if (side == 1) { bid[tick] += signed_q; if (bid[tick] < 0) bid[tick] = 0; }
    else           { ask[tick] += signed_q; if (ask[tick] < 0) ask[tick] = 0; }

    long bbt = 0, bat = LEVELS + 1;
    for (int t = 1; t <= LEVELS; t++) {
        if (bid[t] > 0) bbt = t;
        if (ask[t] > 0 && t < bat) bat = t;
    }
    int has_bid = bbt >= 1, has_ask = bat <= LEVELS, book_ok = has_bid && has_ask;
    long bbp = has_bid ? bbt + (PRICE_MIN - 1) : 0, bbq = has_bid ? bid[bbt] : 0;
    long bap = has_ask ? bat + (PRICE_MIN - 1) : 0, baq = has_ask ? ask[bat] : 0;
    long denom = bbq + baq; if (denom < 1) denom = 1;
    long micro = (bbp * baq + bap * bbq) * 100 / denom;

    long oside = 0, oprice = 0, osize = 0;
    if (book_ok) {
        if (micro > FAIR_100 + EDGE_100) { oside = 2; oprice = bbp; osize = BASE_SIZE; }
        else if (micro < FAIR_100 - EDGE_100) { oside = 1; oprice = bap; osize = BASE_SIZE; }
    }
    if (osize > MAX_ORDER) osize = MAX_ORDER;
    long so = (oside == 1) ? osize : (oside == 2 ? -osize : 0);
    long proj = inventory + so;
    if (proj > POS_CAP || proj < -POS_CAP) { oside = oprice = osize = so = 0; }
    inventory += so;
    if (out) { out[(*outn)++] = oside; out[(*outn)++] = oprice; out[(*outn)++] = osize; }
}

static int cmp_u64(const void *a, const void *b) {
    uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
    return (x > y) - (x < y);
}

int main(int argc, char **argv) {
    long iters = argc > 1 ? atol(argv[1]) : 20000;
    int nmsg = FEED_BYTES / 8;
    /* optional working-set pressure: `--load KB` strides through KB kilobytes between messages, evicting
     * cache the way a loaded trading box does — the mechanism that fattens the CPU tail under market bursts. */
    long load_kb = 0;
    for (int a = 2; a < argc - 1; a++) if (strcmp(argv[a], "--load") == 0) load_kb = atol(argv[a + 1]);
    volatile unsigned char *thrash = NULL; long thrash_n = load_kb * 1024;
    if (thrash_n > 0) { thrash = malloc(thrash_n); if (thrash) for (long k = 0; k < thrash_n; k++) thrash[k] = (unsigned char)k; }

    /* first pass: dump the order stream once so the harness can verify it == the reference */
    memset(bid, 0, sizeof bid); memset(ask, 0, sizeof ask); inventory = 0;
    long shows[512]; int nsh = 0;
    for (int m = 0; m < nmsg; m++) process_msg(DEMO_FEED, m * 8, shows, &nsh);
    if (argc > 2 && strcmp(argv[2], "--emit") == 0) {
        for (int k = 0; k < nsh; k++) printf("%ld%s", shows[k], k + 1 < nsh ? " " : "\n");
        return 0;
    }

    /* calibrate TSC -> ns */
    struct timespec t0, t1; uint64_t c0, c1;
    clock_gettime(CLOCK_MONOTONIC, &t0); c0 = __rdtsc();
    struct timespec sl = {0, 50000000L}; nanosleep(&sl, NULL);
    clock_gettime(CLOCK_MONOTONIC, &t1); c1 = __rdtsc();
    double ns_per_cyc = ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / (double)(c1 - c0);

    long total = iters * nmsg;
    uint64_t *lat = malloc(total * sizeof(uint64_t));
    if (!lat) { fprintf(stderr, "oom\n"); return 1; }

    /* warm up */
    memset(bid, 0, sizeof bid); memset(ask, 0, sizeof ask); inventory = 0;
    for (int w = 0; w < 1000; w++) for (int m = 0; m < nmsg; m++) process_msg(DEMO_FEED, m * 8, NULL, NULL);

    long idx = 0;
    volatile unsigned char sink = 0;
    for (long it = 0; it < iters; it++) {
        memset(bid, 0, sizeof bid); memset(ask, 0, sizeof ask); inventory = 0;
        for (int m = 0; m < nmsg; m++) {
            if (thrash) { for (long k = 0; k < thrash_n; k += 64) sink ^= thrash[k]; }  /* evict working set */
            unsigned aux;
            uint64_t s = __rdtscp(&aux);
            process_msg(DEMO_FEED, m * 8, NULL, NULL);
            uint64_t e = __rdtscp(&aux);
            lat[idx++] = e - s;
        }
    }
    (void)sink;

    qsort(lat, total, sizeof(uint64_t), cmp_u64);
    double mean = 0; for (long k = 0; k < total; k++) mean += lat[k]; mean /= total;
    double var = 0; for (long k = 0; k < total; k++) { double d = lat[k] - mean; var += d * d; } var /= total;
    double stdev_ns = sqrt(var) * ns_per_cyc;
#define P(q) (lat[(long)((q) * (total - 1))] * ns_per_cyc)
    /* the tail curve: latency vs percentile, dense near 100% where the jitter tail lives — the canonical
     * HFT visual (a flat line means no tail; a hockey stick means you get picked off at the worst moment). */
    static const double PCT[] = {0, 0.10, 0.25, 0.50, 0.75, 0.90, 0.95, 0.99, 0.995,
                                 0.999, 0.9995, 0.9999, 0.99995, 0.99999, 1.0};
    int npct = (int)(sizeof(PCT) / sizeof(PCT[0]));
    printf("{\"samples\": %ld, \"ns_per_cyc\": %.6f, "
           "\"p50_ns\": %.1f, \"p99_ns\": %.1f, \"p999_ns\": %.1f, \"p9999_ns\": %.1f, "
           "\"min_ns\": %.1f, \"max_ns\": %.1f, \"mean_ns\": %.1f, \"stdev_ns\": %.1f, "
           "\"tail_ratio_p999_p50\": %.1f, \"curve\": [",
           total, ns_per_cyc,
           P(0.50), P(0.99), P(0.999), P(0.9999), P(0.0), P(1.0), mean * ns_per_cyc, stdev_ns,
           P(0.999) / P(0.50));
    for (int k = 0; k < npct; k++)
        printf("%s[%.5f,%.1f]", k ? "," : "", PCT[k], P(PCT[k]));
    printf("]}\n");
    free(lat);
    return 0;
}
