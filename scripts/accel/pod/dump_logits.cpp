// Dump llama.cpp's full logit vector for the last token of a prompt, as raw f32.
//
// No shipped llama.cpp tool prints the whole vector — llama-eval-callback prints a handful of elements per
// tensor, which is enough to localize a bug but not to measure accuracy. This is the smallest program that
// gets the real thing out, using only the public API, so the comparison is against what llama.cpp actually
// computes rather than against a re-derivation of it.
//
//   g++ -O2 dump_logits.cpp -I<llama>/include -I<llama>/ggml/include -L<llama>/build/bin -lllama \
//       -o dump_logits && ./dump_logits model.gguf out.f32 760 6511 314 9338 369
#include "llama.h"
#include <cstdio>
#include <cstdlib>
#include <vector>

int main(int argc, char** argv) {
    if (argc < 4) { fprintf(stderr, "usage: %s model.gguf out.f32 tok...\n", argv[0]); return 2; }
    std::vector<llama_token> toks;
    for (int i = 3; i < argc; ++i) toks.push_back((llama_token) atoi(argv[i]));

    llama_backend_init();
    llama_model_params mp = llama_model_default_params();
    mp.n_gpu_layers = 99;
    llama_model* model = llama_model_load_from_file(argv[1], mp);
    if (!model) { fprintf(stderr, "load failed\n"); return 1; }

    llama_context_params cp = llama_context_default_params();
    cp.n_ctx = 512;
    cp.n_batch = (uint32_t) toks.size() + 8;
    // Deterministic and comparable: no flash-attention reordering unless explicitly asked for.
    if (const char* fa = getenv("LOGOS_FA")) cp.flash_attn_type = atoi(fa)
        ? LLAMA_FLASH_ATTN_TYPE_ENABLED : LLAMA_FLASH_ATTN_TYPE_DISABLED;
    llama_context* ctx = llama_init_from_model(model, cp);
    if (!ctx) { fprintf(stderr, "ctx failed\n"); return 1; }

    // ONE TOKEN PER DECODE by default. This matters: llama.cpp's batched prefill runs f16 GEMM kernels,
    // while its batch-1 decode runs the int8 MMVQ path — and it is the decode path that generates every
    // token after the prompt. Our program prefills one token at a time through the same kernels it decodes
    // with, so comparing against llama's batched prefill would be comparing against a code path that never
    // produces a generated token. Set LOGOS_BATCHED=1 to use the batched path instead.
    const bool batched = getenv("LOGOS_BATCHED") != nullptr;
    llama_batch batch = llama_batch_init((int32_t) toks.size(), 0, 1);
    if (batched) {
        for (size_t i = 0; i < toks.size(); ++i) {
            batch.token[i] = toks[i];
            batch.pos[i] = (llama_pos) i;
            batch.n_seq_id[i] = 1;
            batch.seq_id[i][0] = 0;
            batch.logits[i] = (int8_t) (i + 1 == toks.size());
        }
        batch.n_tokens = (int32_t) toks.size();
        if (llama_decode(ctx, batch) != 0) { fprintf(stderr, "decode failed\n"); return 1; }
    } else {
        for (size_t i = 0; i < toks.size(); ++i) {
            batch.token[0] = toks[i];
            batch.pos[0] = (llama_pos) i;
            batch.n_seq_id[0] = 1;
            batch.seq_id[0][0] = 0;
            batch.logits[0] = 1;
            batch.n_tokens = 1;
            if (llama_decode(ctx, batch) != 0) { fprintf(stderr, "decode failed at %zu\n", i); return 1; }
        }
    }

    const int n_vocab = llama_vocab_n_tokens(llama_model_get_vocab(model));
    const float* logits = llama_get_logits_ith(ctx, batched ? (int32_t) toks.size() - 1 : 0);
    if (!logits) { fprintf(stderr, "no logits\n"); return 1; }

    FILE* f = fopen(argv[2], "wb");
    if (!f) { fprintf(stderr, "cannot write %s\n", argv[2]); return 1; }
    fwrite(logits, sizeof(float), (size_t) n_vocab, f);
    fclose(f);

    int arg = 0; for (int i = 1; i < n_vocab; ++i) if (logits[i] > logits[arg]) arg = i;
    printf("LLAMA_LOGITS mode=%s n_vocab=%d argmax=%d top=%.6f wrote=%s\n",
           batched ? "batched" : "one-at-a-time", n_vocab, arg, logits[arg], argv[2]);
    return 0;
}
