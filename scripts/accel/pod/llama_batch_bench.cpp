// llama.cpp's BATCHED decode throughput, measured directly against libllama.
//
// llama.cpp's own tools do not cooperate on this checkpoint: `llama-batched-bench` segfaults immediately
// (no output, even at -npl 1) and `llama-server`/`llama-parallel` are not built here. So this drives the
// same public API those tools use — N sequences advancing together through one `llama_decode` per step,
// which is exactly what our batched lane does and what makes batching worth anything: one pass over the
// weights serving N tokens.
//
//   g++ -O2 llama_batch_bench.cpp -I<llama>/include -I<llama>/ggml/include -L<bin> -lllama -o llama_batch_bench
//   ./llama_batch_bench model.gguf <B> <n_gen> <tok...>
#include "llama.h"
#include <cstdio>
#include <cstdlib>
#include <chrono>
#include <vector>

int main(int argc, char** argv) {
    if (argc < 5) { fprintf(stderr, "usage: %s model.gguf B n_gen tok...\n", argv[0]); return 2; }
    const int B     = atoi(argv[2]);
    const int NGEN  = atoi(argv[3]);
    std::vector<llama_token> prompt;
    for (int i = 4; i < argc; ++i) prompt.push_back((llama_token) atoi(argv[i]));

    llama_backend_init();
    llama_model_params mp = llama_model_default_params();
    mp.n_gpu_layers = 99;
    llama_model* model = llama_model_load_from_file(argv[1], mp);
    if (!model) { fprintf(stderr, "load failed\n"); return 1; }

    fprintf(stderr, "CHK model loaded\n");
    llama_context_params cp = llama_context_default_params();
    // Generous and aligned: a hybrid-recurrent model carries per-sequence state, and a tight n_ctx is a
    // plausible way to walk off the end of it.
    cp.n_ctx     = (uint32_t) (B * 512);
    cp.n_batch   = (uint32_t) (B * (prompt.size() + 8));
    cp.n_ubatch  = cp.n_batch;
    cp.n_seq_max = (uint32_t) B;
    llama_context* ctx = llama_init_from_model(model, cp);
    if (!ctx) { fprintf(stderr, "ctx failed (B=%d)\n", B); return 1; }

    fprintf(stderr, "CHK ctx created\n");
    const llama_vocab* vocab = llama_model_get_vocab(model);
    const int n_vocab = llama_vocab_n_tokens(vocab);
    llama_batch batch = llama_batch_init((int32_t) (B * (prompt.size() + 1)), 0, B);

    // ── prefill: every sequence gets the same prompt, so every sequence must generate the same tokens ──
    batch.n_tokens = 0;
    for (int b = 0; b < B; ++b) {
        for (size_t i = 0; i < prompt.size(); ++i) {
            int t = batch.n_tokens++;
            batch.token[t]    = prompt[i];
            batch.pos[t]      = (llama_pos) i;
            batch.n_seq_id[t] = 1;
            batch.seq_id[t][0]= b;
            batch.logits[t]   = (int8_t) (i + 1 == prompt.size());
        }
    }
    fprintf(stderr, "CHK batch filled n_tokens=%d\n", batch.n_tokens);
    if (llama_decode(ctx, batch) != 0) { fprintf(stderr, "prefill decode failed (B=%d)\n", B); return 1; }
    fprintf(stderr, "CHK prefill decoded\n");

    std::vector<llama_token> cur(B);
    // `llama_get_logits_ith` indexes the BATCH, not the sequence: only the tokens that set `logits = 1` have
    // a row, and here those are the LAST prompt token of each sequence — positions 4, 9, 14, 19 for B=4, not
    // 0..B-1. Asking for a row that was never requested returns nullptr, and dereferencing it is the
    // segfault this program was dying on.
    for (int b = 0; b < B; ++b) {
        const int32_t row = (int32_t) (b * prompt.size() + prompt.size() - 1);
        const float* lg = llama_get_logits_ith(ctx, row);
        if (!lg) { fprintf(stderr, "no logits at batch row %d (B=%d)\n", row, B); return 1; }
        int arg = 0; for (int i = 1; i < n_vocab; ++i) if (lg[i] > lg[arg]) arg = i;
        cur[b] = arg;
    }
    fprintf(stderr, "CHK logits read\n");
    printf("LLAMA_SEQ0_FIRST_TOKEN %d\n", cur[0]);
    fflush(stdout);

    // ── decode: one token per sequence per step, all sequences sharing the step ──
    double decode_sec = 0.0;
    int pos = (int) prompt.size();
    for (int step = 0; step < NGEN; ++step) {
        batch.n_tokens = 0;
        for (int b = 0; b < B; ++b) {
            int t = batch.n_tokens++;
            batch.token[t]    = cur[b];
            batch.pos[t]      = (llama_pos) pos;
            batch.n_seq_id[t] = 1;
            batch.seq_id[t][0]= b;
            batch.logits[t]   = 1;
        }
        auto d0 = std::chrono::high_resolution_clock::now();
        if (llama_decode(ctx, batch) != 0) { fprintf(stderr, "decode failed at step %d (B=%d)\n", step, B); return 1; }
        llama_synchronize(ctx);          // the decode is async; without this we time the enqueue
        auto d1 = std::chrono::high_resolution_clock::now();
        decode_sec += std::chrono::duration<double>(d1 - d0).count();
        for (int b = 0; b < B; ++b) {
            const float* lg = llama_get_logits_ith(ctx, b);   // decode batches ARE one row per sequence
            if (!lg) { fprintf(stderr, "no logits at row %d during decode (B=%d)\n", b, B); return 1; }
            int arg = 0; for (int i = 1; i < n_vocab; ++i) if (lg[i] > lg[arg]) arg = i;
            cur[b] = arg;
        }
        ++pos;
    }
    const double sec = decode_sec;   // model time only, host argmax excluded

    printf("LLAMA_BATCH %d  PER_SEQ_TOK_S %.2f  AGGREGATE_TOK_S %.2f  (%.3f ms/step)\n",
           B, NGEN / sec, (double) B * NGEN / sec, 1000.0 * sec / NGEN);
    return 0;
}
