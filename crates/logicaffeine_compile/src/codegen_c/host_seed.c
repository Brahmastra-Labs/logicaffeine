/* The seed a host C-family run schedules under: `LOGOS_SEED` read as the Rust tier reads it
   (`logicaffeine_system::concurrency` — trimmed, then a u64 with an optional `+`); `otherwise` when it is
   unset or does not parse. */
static int logos_seed_space(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\v' || c == '\f'; }

static unsigned long long logos_env_seed(unsigned long long otherwise) {
    const char *s = getenv("LOGOS_SEED");
    if (!s) return otherwise;
    while (logos_seed_space(*s)) s++;
    if (*s == '+') s++;
    unsigned long long v = 0;
    int digits = 0;
    for (; *s >= '0' && *s <= '9'; s++, digits++) {
        unsigned long long d = (unsigned long long)(*s - '0');
        if (v > (~0ULL - d) / 10) return otherwise;
        v = v * 10 + d;
    }
    while (logos_seed_space(*s)) s++;
    return (digits && !*s) ? v : otherwise;
}
