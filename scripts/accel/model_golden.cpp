// The llama.cpp golden for a checkpoint: for each prompt, its token ids, the top logits after it, and the greedy
// continuation — computed by the pinned llama.cpp on the CPU alone (no GPU layers, and no operation offloaded: a
// llama.cpp built with CUDA otherwise runs every operation on a batch of 32 or more tokens on the GPU, at the GPU's
// precision), at a fixed thread count, so a golden is reproducible on this box and independent of what else holds the
// GPU. Built and run by scripts/accel/model-golden.sh, which also hides every GPU from it.
//
//   model_golden <gguf> <prompts> <out.json> <n_gen> <threads> <flash:0|1|auto> <top> <kv:f16|f32> [batch:0|1] [device:cpu|cuda]
//
// With device cuda it runs as llama-bench runs a checkpoint instead (`-ngl 99`): every layer on the GPU, operations
// offloaded, llama-bench's batches (2048 tokens, micro-batches of 512), flash attention as given (`auto`, llama-bench's default, lets llama.cpp choose) — llama.cpp's own GPU run,
// whose agreement with the f32 golden the scoreboard records beside its speed. The JSON says so ("device": "cuda").
//
// <prompts> holds the prompts separated by "\n__prompt__\n". The JSON records the llama.cpp commit the script
// passes in LLAMA_COMMIT, the settings, and per prompt: text, tokens, top [[id, logit]…], continuation ids and text.
//
// With batch 1 every prompt is a sequence of its own in one context (its own cache cells and recurrent state): each
// prefilled alone, then every unfinished sequence decoded one token a step in a single batch, so the weights stream
// once a step for the whole corpus rather than once a step for each prompt. It is the run for an f32 copy whose
// weights do not fit in memory, read again from disk on every pass otherwise; it refuses any other checkpoint, whose
// quantized products llama.cpp computes with other kernels in a batch. Batched f32 products round differently in the
// last bits, so the JSON says it was batched ("batch": the sequences in a step).
#include "llama.h"

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <numeric>
#include <sstream>
#include <string>
#include <vector>

// The length of the well-formed UTF-8 sequence starting at s[i], or 0 where none does: then `*sub` is the length of
// the maximal subpart there (the lead byte and the continuation bytes that still fit it), which one U+FFFD replaces.
static size_t utf8_at(const std::string & s, size_t i, size_t * sub) {
    const unsigned char c = (unsigned char) s[i];
    if (c < 0x80) return 1;
    size_t n;
    unsigned char lo = 0x80, hi = 0xBF;
    if (c >= 0xC2 && c <= 0xDF) {
        n = 2;
    } else if (c >= 0xE0 && c <= 0xEF) {
        n = 3;
        if (c == 0xE0) lo = 0xA0;
        if (c == 0xED) hi = 0x9F;
    } else if (c >= 0xF0 && c <= 0xF4) {
        n = 4;
        if (c == 0xF0) lo = 0x90;
        if (c == 0xF4) hi = 0x8F;
    } else {
        *sub = 1;
        return 0;
    }
    size_t k = 1;
    for (; k < n && i + k < s.size(); ++k) {
        const unsigned char b = (unsigned char) s[i + k];
        if (b < (k == 1 ? lo : 0x80) || b > (k == 1 ? hi : 0xBF)) break;
    }
    if (k == n) return n;
    *sub = k;
    return 0;
}

// A JSON string body, valid UTF-8 whatever the bytes: a continuation of n_gen tokens can stop inside a character, so
// every ill-formed or cut-short sequence is written as U+FFFD, a maximal subpart at a time, and every well-formed one
// as it is.
static std::string json_escape(const std::string & s) {
    std::string out;
    for (size_t i = 0; i < s.size();) {
        size_t sub = 0;
        const size_t n = utf8_at(s, i, &sub);
        if (n == 0) {
            out += "\xEF\xBF\xBD";
            i += sub;
            continue;
        }
        if (n > 1) {
            out.append(s, i, n);
            i += n;
            continue;
        }
        const unsigned char c = (unsigned char) s[i++];
        switch (c) {
            case '"': out += "\\\""; break;
            case '\\': out += "\\\\"; break;
            case '\n': out += "\\n"; break;
            case '\r': out += "\\r"; break;
            case '\t': out += "\\t"; break;
            default:
                if (c < 0x20) {
                    char buf[8];
                    snprintf(buf, sizeof buf, "\\u%04x", c);
                    out += buf;
                } else {
                    out += (char) c;
                }
        }
    }
    return out;
}

static std::string piece(const llama_vocab * v, const std::vector<llama_token> & toks) {
    std::vector<char> buf(4096);
    int n = llama_detokenize(v, toks.data(), (int) toks.size(), buf.data(), (int) buf.size(), false, false);
    if (n < 0) {
        buf.resize(-n);
        n = llama_detokenize(v, toks.data(), (int) toks.size(), buf.data(), (int) buf.size(), false, false);
    }
    return std::string(buf.data(), n);
}

int main(int argc, char ** argv) {
    if (argc < 9 || argc > 11) {
        fprintf(stderr, "usage: model_golden <gguf> <prompts> <out.json> <n_gen> <threads> <flash:0|1|auto> <top> <kv:f16|f32> [batch:0|1] [device:cpu|cuda]\n");
        return 2;
    }
    const bool batched = argc >= 10 && atoi(argv[9]) != 0;
    const std::string device = argc == 11 ? argv[10] : "cpu";
    if (device != "cpu" && device != "cuda") {
        fprintf(stderr, "the device is cpu or cuda\n");
        return 2;
    }
    const bool gpu = device == "cuda";
    const std::string flash_arg = argv[6];
    const std::string kv = argv[8];
    if (kv != "f16" && kv != "f32") {
        fprintf(stderr, "the KV cache is f16 or f32\n");
        return 2;
    }
    const std::string model_path = argv[1], prompts_path = argv[2], out_path = argv[3];
    const int n_gen = atoi(argv[4]), threads = atoi(argv[5]), flash = flash_arg == "auto" ? -1 : atoi(argv[6]), top = atoi(argv[7]);

    std::ifstream pin(prompts_path, std::ios::binary);
    std::stringstream ss;
    ss << pin.rdbuf();
    const std::string raw = ss.str();
    const std::string sep = "\n__prompt__\n";
    std::vector<std::string> prompts;
    for (size_t pos = 0; pos < raw.size();) {
        size_t next = raw.find(sep, pos);
        if (next == std::string::npos) {
            prompts.push_back(raw.substr(pos));
            break;
        }
        prompts.push_back(raw.substr(pos, next - pos));
        pos = next + sep.size();
    }

    llama_log_set([](enum ggml_log_level, const char *, void *) {}, nullptr);
    llama_backend_init();
    auto mp = llama_model_default_params();
    mp.n_gpu_layers = gpu ? 999 : 0;
    llama_model * model = llama_model_load_from_file(model_path.c_str(), mp);
    if (!model) {
        fprintf(stderr, "the model does not load\n");
        return 3;
    }
    const llama_vocab * vocab = llama_model_get_vocab(model);
    const int n_vocab = llama_vocab_n_tokens(vocab);
    if (batched) {
        // A quantized checkpoint's batched products are not its one-prompt-at-a-time ones (llama.cpp takes other
        // kernels for them), so only an all-f32 copy (general.file_type 0) is run batched.
        char ftype[16] = "";
        llama_model_meta_val_str(model, "general.file_type", ftype, sizeof ftype);
        if (std::string(ftype) != "0") {
            fprintf(stderr, "batch 1 runs only an all-f32 checkpoint (general.file_type 0), not file type %s\n", ftype);
            return 2;
        }
    }
    std::vector<std::vector<llama_token>> tokens(prompts.size());
    for (size_t p = 0; p < prompts.size(); ++p) {
        const std::string & text = prompts[p];
        int n = llama_tokenize(vocab, text.data(), (int) text.size(), nullptr, 0, true, false);
        tokens[p].resize(n < 0 ? -n : n);
        llama_tokenize(vocab, text.data(), (int) text.size(), tokens[p].data(), (int) tokens[p].size(), true, false);
    }

    auto cp = llama_context_default_params();
    cp.n_ctx = 4096;
    // On the GPU, llama-bench's batches (`-b 2048 -ub 512`): a 4096-token micro-batch's compute buffer does not fit
    // a 16B mixture beside its weights on a 12 GB card.
    cp.n_batch = gpu ? 2048 : 4096;
    cp.n_ubatch = gpu ? 512 : 4096;
    if (batched) {
        // Every sequence's prompt and continuation in one cache the sequences share; every prompt prefilled in one batch.
        uint32_t cells = 0, longest = 0;
        for (const auto & t : tokens) {
            cells += (uint32_t) t.size() + (uint32_t) n_gen;
            longest = std::max(longest, (uint32_t) t.size());
        }
        cp.n_ctx = std::max(cells, 256u);
        cp.n_batch = cp.n_ubatch = std::max(longest, (uint32_t) prompts.size());
        cp.n_seq_max = (uint32_t) prompts.size();
        cp.kv_unified = true;
    }
    cp.n_threads = threads;
    cp.n_threads_batch = threads;
    cp.flash_attn_type = flash < 0 ? LLAMA_FLASH_ATTN_TYPE_AUTO : flash ? LLAMA_FLASH_ATTN_TYPE_ENABLED : LLAMA_FLASH_ATTN_TYPE_DISABLED;
    cp.no_perf = true;
    cp.op_offload = gpu;
    cp.type_k = kv == "f32" ? GGML_TYPE_F32 : GGML_TYPE_F16;
    cp.type_v = cp.type_k;
    llama_context * ctx = llama_init_from_model(model, cp);
    if (!ctx) {
        fprintf(stderr, "the context does not initialize\n");
        return 4;
    }

    const char * commit = getenv("LLAMA_COMMIT");
    std::ofstream out(out_path, std::ios::binary);
    out << "{\n  \"model\": \"" << json_escape(model_path) << "\",\n";
    out << "  \"llama_cpp\": \"" << (commit ? commit : "") << "\",\n";
    out << "  \"device\": \"" << device << "\",\n  \"op_offload\": " << (gpu ? "true" : "false") << ",\n  \"kv\": \"" << kv << "\",\n  \"threads\": " << threads << ",\n  \"flash_attn\": " << (flash < 0 ? "\"auto\"" : flash ? "true" : "false")
        << ",\n  \"n_batch\": " << cp.n_batch << ",\n  \"n_ubatch\": " << cp.n_ubatch << ",\n  \"n_gen\": " << n_gen << ",\n";
    if (batched) out << "  \"batch\": " << prompts.size() << ",\n";
    out << "  \"prompts\": [\n";

    auto top_of = [&](const float * logits, std::vector<std::pair<int, float>> & keep) {
        std::vector<int> order(n_vocab);
        std::iota(order.begin(), order.end(), 0);
        std::partial_sort(order.begin(), order.begin() + top, order.end(), [&](int a, int b) {
            return logits[a] > logits[b] || (logits[a] == logits[b] && a < b);
        });
        for (int k = 0; k < top; ++k) {
            keep.push_back({order[k], logits[order[k]]});
        }
        return (llama_token) order[0];
    };
    auto argmax = [&](const float * logits) { return (llama_token) (std::max_element(logits, logits + n_vocab) - logits); };
    auto emit = [&](size_t p, const std::vector<std::pair<int, float>> & best, const std::vector<llama_token> & gen) {
        const std::vector<llama_token> & toks = tokens[p];
        out << "    {\n      \"text\": \"" << json_escape(prompts[p]) << "\",\n      \"tokens\": [";
        for (size_t i = 0; i < toks.size(); ++i) out << (i ? ", " : "") << toks[i];
        out << "],\n      \"top\": [";
        for (size_t i = 0; i < best.size(); ++i) {
            char buf[64];
            snprintf(buf, sizeof buf, "%.9g", best[i].second);
            out << (i ? ", " : "") << "[" << best[i].first << ", " << buf << "]";
        }
        out << "],\n      \"continuation\": [";
        for (size_t i = 0; i < gen.size(); ++i) out << (i ? ", " : "") << gen[i];
        out << "],\n      \"continuation_text\": \"" << json_escape(piece(vocab, gen)) << "\"\n    }"
            << (p + 1 < prompts.size() ? "," : "") << "\n";
    };

    if (!batched) {
        for (size_t p = 0; p < prompts.size(); ++p) {
            llama_memory_clear(llama_get_memory(ctx), true);
            std::vector<llama_token> & toks = tokens[p];
            if (llama_decode(ctx, llama_batch_get_one(toks.data(), (int) toks.size())) != 0) {
                fprintf(stderr, "prompt %zu does not decode\n", p);
                return 5;
            }
            std::vector<std::pair<int, float>> best;
            llama_token next = top_of(llama_get_logits_ith(ctx, -1), best);
            std::vector<llama_token> gen;
            for (int i = 0; i < n_gen; ++i) {
                gen.push_back(next);
                if (llama_vocab_is_eog(vocab, next) || i + 1 == n_gen) {
                    break;
                }
                if (llama_decode(ctx, llama_batch_get_one(&next, 1)) != 0) {
                    fprintf(stderr, "prompt %zu step %d does not decode\n", p, i);
                    return 6;
                }
                next = argmax(llama_get_logits_ith(ctx, -1));
            }
            emit(p, best, gen);
        }
    } else {
        const size_t np = prompts.size();
        std::vector<std::vector<std::pair<int, float>>> best(np);
        std::vector<std::vector<llama_token>> gen(np);
        std::vector<llama_token> next(np);
        std::vector<llama_pos> pos(np);
        std::vector<bool> live(np, true);
        llama_memory_clear(llama_get_memory(ctx), true);
        for (size_t p = 0; p < np; ++p) {
            llama_batch b = llama_batch_init((int) tokens[p].size(), 0, 1);
            for (size_t i = 0; i < tokens[p].size(); ++i) {
                b.token[b.n_tokens] = tokens[p][i];
                b.pos[b.n_tokens] = (llama_pos) i;
                b.n_seq_id[b.n_tokens] = 1;
                b.seq_id[b.n_tokens][0] = (llama_seq_id) p;
                b.logits[b.n_tokens] = i + 1 == tokens[p].size();
                b.n_tokens++;
            }
            const int r = llama_decode(ctx, b);
            llama_batch_free(b);
            if (r != 0) {
                fprintf(stderr, "prompt %zu does not decode\n", p);
                return 5;
            }
            next[p] = top_of(llama_get_logits_ith(ctx, -1), best[p]);
            pos[p] = (llama_pos) tokens[p].size();
        }
        for (int step = 0; step < n_gen; ++step) {
            std::vector<size_t> rows;
            for (size_t p = 0; p < np; ++p) {
                if (!live[p]) continue;
                gen[p].push_back(next[p]);
                if (llama_vocab_is_eog(vocab, next[p]) || step + 1 == n_gen) {
                    live[p] = false;
                } else {
                    rows.push_back(p);
                }
            }
            if (rows.empty()) break;
            llama_batch b = llama_batch_init((int) rows.size(), 0, 1);
            for (size_t p : rows) {
                b.token[b.n_tokens] = next[p];
                b.pos[b.n_tokens] = pos[p]++;
                b.n_seq_id[b.n_tokens] = 1;
                b.seq_id[b.n_tokens][0] = (llama_seq_id) p;
                b.logits[b.n_tokens] = true;
                b.n_tokens++;
            }
            const int r = llama_decode(ctx, b);
            if (r != 0) {
                llama_batch_free(b);
                fprintf(stderr, "step %d does not decode\n", step);
                return 6;
            }
            for (size_t i = 0; i < rows.size(); ++i) next[rows[i]] = argmax(llama_get_logits_ith(ctx, (int) i));
            llama_batch_free(b);
        }
        for (size_t p = 0; p < np; ++p) emit(p, best[p], gen[p]);
    }
    out << "  ]\n}\n";
    llama_free(ctx);
    llama_model_free(model);
    return 0;
}
