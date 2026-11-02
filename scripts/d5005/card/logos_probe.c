/* logos_probe: logos_step with LOGOS_PROBE (see probe()). Run a model on the decode engine: the program `ofs::write_engine_program` wrote — the memory image, the engine's
   configuration and tables, the embedding table — loaded onto the card, then greedy decoding one token a step from a
   prompt: each step the row table and the token's embedding written into the engine, the step run, and, once past
   the prompt, the logits read back and their arg-max the next token. Each step's position, token and cycles on stderr
   as it completes; one JSON object on stdout once decoding ends: every token, each step's cycles and the engine's and
   card port's faults. The exit status is 0 only when every step ran without one, 2 when the program or the card cannot
   be had.

     logos_step [--inject] <program directory> <tokens to generate> <prompt token>...

   With --inject, on SECDED lines, every line the image load writes carries one flipped bit, which every read of it
   corrects; the record then carries each channel's words corrected and lost. Built against the OPAE backend it runs on
   the card; against a simulation backend, inside the RTL. */
#include "logos_core.h"

#include <inttypes.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#ifndef LOGOS_PROGRAM_MAIN
#define LOGOS_PROGRAM_MAIN main
#endif

/* Host lines one copy moves. */
#define LOGOS_STEP_CHUNK 4096u

typedef struct {
    uint64_t knobs, space, base, granule_log2, job_words, residual_beat, residual_beats, logits_beat, logits_beats;
    uint64_t tokens, hidden, vocab, ring_rows, regions, region, sequences, ctx;
    uint64_t channel_lines[LOGOS_D5005_BANKS];
    char streaming[256];
} program;

static int failures;
static uint64_t tag = 1;

static void expect(int ok, const char *what) {
    if (!ok) {
        ++failures;
        fprintf(stderr, "logos_step: %s\n", what);
    }
}

static logos_record run(logos_dev *dev, uint8_t op, uint64_t a0, uint64_t a1, uint64_t a2) {
    uint64_t t = tag++;
    logos_record r = logos_core_run(dev, op, t, a0, a1, a2);
    expect((r.status >> 8) == t && (r.status & 0xff) == LOGOS_D5005_STATUS_OK, "a command does not complete");
    return r;
}

static int read_program(const char *dir, program *p) {
    char path[4096], key[64];
    snprintf(path, sizeof path, "%s/engine.txt", dir);
    FILE *f = fopen(path, "r");
    if (!f) return -1;
    memset(p, 0, sizeof *p);
    int fields = 0;
    while (fscanf(f, "%63s", key) == 1) {
        uint64_t *one = NULL, *two = NULL;
        if (!strcmp(key, "knobs")) one = &p->knobs;
        else if (!strcmp(key, "space")) one = &p->space;
        else if (!strcmp(key, "base")) one = &p->base;
        else if (!strcmp(key, "granule_log2")) one = &p->granule_log2;
        else if (!strcmp(key, "job_words")) one = &p->job_words;
        else if (!strcmp(key, "residual")) { one = &p->residual_beat; two = &p->residual_beats; }
        else if (!strcmp(key, "logits")) { one = &p->logits_beat; two = &p->logits_beats; }
        else if (!strcmp(key, "tokens")) one = &p->tokens;
        else if (!strcmp(key, "hidden")) one = &p->hidden;
        else if (!strcmp(key, "vocab")) one = &p->vocab;
        else if (!strcmp(key, "ring_rows")) one = &p->ring_rows;
        else if (!strcmp(key, "regions")) one = &p->regions;
        else if (!strcmp(key, "region")) one = &p->region;
        else if (!strcmp(key, "sequences")) one = &p->sequences;
        else if (!strcmp(key, "ctx")) one = &p->ctx;
        else if (!strcmp(key, "channels")) {
            for (int k = 0; k < LOGOS_D5005_BANKS; ++k)
                if (fscanf(f, "%" SCNu64, &p->channel_lines[k]) != 1) { fclose(f); return -1; }
            ++fields;
            continue;
        } else if (!strcmp(key, "streaming")) {
            if (fscanf(f, "%255s", p->streaming) != 1) { fclose(f); return -1; }
            continue;
        } else { fclose(f); return -1; }
        if (fscanf(f, "%" SCNu64, one) != 1 || (two && fscanf(f, "%" SCNu64, two) != 1)) { fclose(f); return -1; }
        ++fields;
    }
    fclose(f);
    return fields == 16 && p->ring_rows > 0 && p->hidden <= p->residual_beats * 16 && p->vocab <= p->logits_beats * 16 ? 0 : -1;
}

/* A file's bytes, its length in `n`. */
static uint8_t *slurp(const char *dir, const char *name, size_t *n) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *b = malloc(len > 0 ? (size_t)len : 1);
    *n = len > 0 && b && fread(b, 1, (size_t)len, f) == (size_t)len ? (size_t)len : 0;
    fclose(f);
    if (len > 0 && *n == 0) { free(b); return NULL; }
    return b;
}

/* A file mapped read-only, its length in `n`: the memory image and the embeddings are larger than a host's memory may
   be, so they are read as the card takes them. */
static const uint8_t *map_file(const char *dir, const char *name, size_t *n) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    *n = 0;
    int fd = open(path, O_RDONLY);
    if (fd < 0) return NULL;
    struct stat st;
    if (fstat(fd, &st) || st.st_size <= 0) {
        close(fd);
        return NULL;
    }
    void *m = mmap(NULL, (size_t)st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (m == MAP_FAILED) return NULL;
    *n = (size_t)st.st_size;
    return m;
}

/* `lines` host lines of `bytes` into the engine's table `table` from entry `first`, through the host buffer. */
static void table_write(logos_dev *dev, uint8_t *buf, uint64_t io, int table, uint64_t first, const uint8_t *bytes, uint64_t lines) {
    for (uint64_t at = 0; at < lines; at += LOGOS_STEP_CHUNK) {
        uint64_t n = lines - at < LOGOS_STEP_CHUNK ? lines - at : LOGOS_STEP_CHUNK;
        memcpy(buf, bytes + at * 64, (size_t)n * 64);
        run(dev, LOGOS_D5005_OP_ENGINE_TABLE, io >> 6, (uint64_t)table << 32 | (first + at), n);
    }
}

/* LOGOS_PROBE names a directory: after each position from the prompt's last on, the logits read in one STORE_READ
   (as logos_step reads them) go to single-<pos>.bin, and the store's every beat, read in chunks of the host buffer (as
   logos_trace reads it), to store-<pos>.bin; the arg-max of each is printed. */
static uint64_t argmax(const float *x, uint64_t n);
static void probe(logos_dev *dev, uint8_t *buf, uint64_t io, uint64_t buf_lines, const program *p, const char *dir,
                  uint64_t pos) {
    char path[4096];
    uint64_t beats = p->logits_beat + p->logits_beats;
    run(dev, LOGOS_D5005_OP_STORE_READ, io >> 6, p->logits_beat, p->logits_beats);
    uint64_t one = argmax((const float *)buf, p->vocab);
    snprintf(path, sizeof path, "%s/single-%" PRIu64 ".bin", dir, pos);
    FILE *o = fopen(path, "wb");
    fwrite(buf, 64, (size_t)p->logits_beats, o);
    fclose(o);
    float *logits = malloc((size_t)p->logits_beats * 64);
    snprintf(path, sizeof path, "%s/store-%" PRIu64 ".bin", dir, pos);
    o = fopen(path, "wb");
    for (uint64_t at = 0; at < beats; at += 4096) {
        uint64_t n = beats - at < 4096 ? beats - at : 4096;
        run(dev, LOGOS_D5005_OP_STORE_READ, io >> 6, at, n);
        fwrite(buf, 64, (size_t)n, o);
        for (uint64_t k = 0; k < n; ++k)
            if (at + k >= p->logits_beat) memcpy((uint8_t *)logits + (at + k - p->logits_beat) * 64, buf + k * 64, 64);
    }
    fclose(o);
    uint64_t chunked = argmax(logits, p->vocab);
    free(logits);
    fprintf(stderr, "logos_probe: position %" PRIu64 ": one read's arg-max %" PRIu64 ", chunked reads' %" PRIu64 "\n", pos, one,
            chunked);
    (void)buf_lines;
}

/* The first strictly largest logit that is a number, as numerics::argmax picks it. */
static uint64_t argmax(const float *x, uint64_t n) {
    uint64_t best = 0;
    int found = 0;
    for (uint64_t i = 0; i < n; ++i)
        if (!isnan(x[i]) && (!found || x[i] > x[best])) { best = i; found = 1; }
    return best;
}

int LOGOS_PROGRAM_MAIN(int argc, char **argv) {
    int inject = argc > 1 && !strcmp(argv[1], "--inject");
    if (inject) {
        --argc;
        ++argv;
    }
    if (argc < 4 || (inject && !LOGOS_D5005_SECDED)) {
        fprintf(stderr, "usage: logos_step [--inject] <program directory> <tokens to generate> <prompt token>..., --inject on SECDED lines\n");
        return 2;
    }
    const char *dir = argv[1];
    uint64_t generate = strtoull(argv[2], NULL, 0), prompt_n = (uint64_t)(argc - 3);
    program p;
    if (read_program(dir, &p)) {
        fprintf(stderr, "logos_step: %s/engine.txt is no engine program\n", dir);
        return 2;
    }
    if (strcmp(p.streaming, LOGOS_D5005_STREAMING)) {
        fprintf(stderr, "logos_step: %s is a program for the streaming unit \"%s\", this build's is \"%s\"\n", dir, p.streaming,
                LOGOS_D5005_STREAMING);
        return 2;
    }
    if (prompt_n + generate > p.ctx) {
        fprintf(stderr, "logos_step: %" PRIu64 " tokens past the %" PRIu64 " positions a sequence reaches\n", prompt_n + generate, p.ctx);
        return 2;
    }
    size_t embed_n = 0;
    const uint8_t *embed = map_file(dir, "embed.bin", &embed_n);
    if (!embed || embed_n != p.tokens * p.hidden * 4) {
        fprintf(stderr, "logos_step: embed.bin is not %" PRIu64 " embeddings of %" PRIu64 " elements\n", p.tokens, p.hidden);
        return 2;
    }
    logos_dev *dev = logos_open(LOGOS_D5005_AFU_ID);
    if (!dev) return 2;
    uint64_t io = 0, buf_lines = LOGOS_STEP_CHUNK > p.logits_beats ? LOGOS_STEP_CHUNK : p.logits_beats;
    uint8_t *buf = logos_buffer(dev, (size_t)buf_lines * 64, &io);
    if (!buf) return 2;

    run(dev, LOGOS_D5005_OP_ENGINE_CONFIG, 1 | p.knobs << 16, p.space, p.base | p.granule_log2 << 56);
    static const struct { int table; const char *file; } tables[] = {
        {0, "table0.bin"}, {2, "table2.bin"}, {3, "table3.bin"}, {4, "table4.bin"}, {5, "table5.bin"}};
    for (size_t k = 0; k < sizeof tables / sizeof tables[0]; ++k) {
        size_t n = 0;
        uint8_t *bytes = slurp(dir, tables[k].file, &n);
        if (!bytes || n % 64) {
            fprintf(stderr, "logos_step: %s is not whole 64-byte entries\n", tables[k].file);
            return 2;
        }
        table_write(dev, buf, io, tables[k].table, 0, bytes, n / 64);
        free(bytes);
    }
    if (inject)
        for (int k = 0; k < LOGOS_D5005_BANKS; ++k)
            run(dev, LOGOS_D5005_OP_INJECT, (uint64_t)1 << k, 5u << 16 | 5u, p.channel_lines[k]);
    for (int k = 0; k < LOGOS_D5005_BANKS; ++k) {
        char name[32];
        size_t n = 0;
        snprintf(name, sizeof name, "channel%d.bin", k);
        const uint8_t *bytes = p.channel_lines[k] ? map_file(dir, name, &n) : NULL;
        if (n != p.channel_lines[k] * LOGOS_D5005_MEM_LINE_BYTES || (n && !bytes)) {
            fprintf(stderr, "logos_step: %s is not %" PRIu64 " lines\n", name, p.channel_lines[k]);
            return 2;
        }
        for (uint64_t at = 0; at < p.channel_lines[k]; at += LOGOS_STEP_CHUNK) {
            uint64_t lines = p.channel_lines[k] - at < LOGOS_STEP_CHUNK ? p.channel_lines[k] - at : LOGOS_STEP_CHUNK;
            memcpy(buf, bytes + at * LOGOS_D5005_MEM_LINE_BYTES, (size_t)lines * LOGOS_D5005_MEM_LINE_BYTES);
            run(dev, LOGOS_D5005_OP_HOST_TO_MEM, io >> 6, (uint64_t)k << 56 | at, lines);
        }
        if (bytes) munmap((void *)bytes, n);
    }

    uint64_t total = prompt_n + generate, token = strtoull(argv[3], NULL, 0), ran = 0;
    uint64_t *cycles = calloc((size_t)total, sizeof *cycles), *tokens = calloc((size_t)total, sizeof *tokens);
    int faults = 0;
    for (uint64_t pos = 0; pos < total; ++pos) {
        if (token >= p.tokens) {
            fprintf(stderr, "logos_step: token %" PRIu64 " past the embedding table\n", token);
            return 2;
        }
        tokens[pos] = token;
        ran = pos + 1;
        uint8_t row[64] = {0};
        uint64_t region = p.regions, ring = pos % p.ring_rows;
        memcpy(row, &region, 4);
        memcpy(row + 4, &pos, 4);
        row[8] = (uint8_t)ring;
        table_write(dev, buf, io, 1, 0, row, 1);
        uint8_t *e = calloc((size_t)p.residual_beats, 64);
        memcpy(e, embed + token * p.hidden * 4, (size_t)p.hidden * 4);
        table_write(dev, buf, io, 6, p.residual_beat, e, p.residual_beats);
        free(e);
        logos_record step = run(dev, LOGOS_D5005_OP_ENGINE_RUN, 0, p.job_words, 0);
        cycles[pos] = step.result1;
        fprintf(stderr, "logos_step: position %" PRIu64 " of %" PRIu64 ", token %" PRIu64 ": %" PRIu64 " cycles\n", pos, total,
                token, step.result1);
        if (step.result0 & 3) {
            ++faults;
            fprintf(stderr, "logos_step: position %" PRIu64 ": the %s faulted at word %" PRIu64 "\n", pos,
                    step.result0 & 1 ? "engine" : "card port", step.result0 >> 32);
            break;
        }
        if (pos + 1 >= prompt_n && getenv("LOGOS_PROBE")) probe(dev, buf, io, buf_lines, &p, getenv("LOGOS_PROBE"), pos);
        if (pos + 1 < prompt_n) {
            token = strtoull(argv[4 + pos], NULL, 0);
        } else if (pos + 1 < total) {
            run(dev, LOGOS_D5005_OP_STORE_READ, io >> 6, p.logits_beat, p.logits_beats);
            token = argmax((const float *)buf, p.vocab);
        }
    }
    printf("{\"tokens\": [");
    for (uint64_t pos = 0; pos < ran; ++pos) printf("%s%" PRIu64, pos ? ", " : "", tokens[pos]);
    printf("], \"prompt\": %" PRIu64 ", \"cycles\": [", prompt_n);
    for (uint64_t pos = 0; pos < total; ++pos) printf("%s%" PRIu64, pos ? ", " : "", cycles[pos]);
    printf("], \"faults\": %d", faults);
    if (inject) {
        printf(", \"secded\": [");
        for (int k = 0; k < LOGOS_D5005_BANKS; ++k) {
            logos_record c = run(dev, LOGOS_D5005_OP_ECC_COUNTS, (uint64_t)k, 0, 0);
            printf("%s{\"corrected\": %" PRIu64 ", \"uncorrectable\": %" PRIu64 "}", k ? ", " : "", c.result0 & 0xffffffffu, c.result0 >> 32);
        }
        printf("]");
    }
    printf(", \"checks_failed\": %d}\n", failures);
    fflush(stdout);
    free(cycles);
    free(tokens);
    munmap((void *)embed, embed_n);
    logos_buffer_free(dev, buf);
    logos_close(dev);
    return failures || faults ? 1 : 0;
}
