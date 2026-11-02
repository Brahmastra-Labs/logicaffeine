/* ========== Text building ========== */

/* A text a program appends to in place is a builder: its length, its capacity and whether every byte of it is
   ASCII sit in a header just before its characters, so an append copies only what it appends and the buffer grows
   by doubling, and its length and its characters are read in constant time. Every other text is a plain C
   string. */
typedef struct { size_t len, cap; int ascii; } logos_text_hdr;

__attribute__((unused)) static int logos_text_is_ascii(const char *s, size_t n) {
    for (size_t i = 0; i < n; i++)
        if ((unsigned char)s[i] >= 0x80) return 0;
    return 1;
}

__attribute__((unused)) static char *logos_text_builder(const char *text, int64_t reserve) {
    size_t len = strlen(text);
    size_t cap = reserve > 0 && (size_t)reserve > len ? (size_t)reserve : len;
    logos_text_hdr *h = (logos_text_hdr *)lc_gc_alloc(sizeof(logos_text_hdr) + cap + 1);
    h->len = len;
    h->cap = cap;
    h->ascii = logos_text_is_ascii(text, len);
    char *s = (char *)(h + 1);
    memcpy(s, text, len + 1);
    return s;
}

__attribute__((unused)) static size_t logos_text_builder_len(const char *s) {
    return ((const logos_text_hdr *)s - 1)->len;
}

/* Append `text` to the builder `*s`; `text` may be part of the builder itself. */
__attribute__((unused)) static void logos_text_append(char **s, const char *text) {
    logos_text_hdr *h = (logos_text_hdr *)*s - 1;
    size_t tl = strlen(text);
    char *inside = (text >= *s && text <= *s + h->len) ? lc_gc_strdup(text) : NULL;
    if (inside) text = inside;
    if (h->len + tl > h->cap) {
        size_t cap = h->cap * 2;
        if (cap < h->len + tl) cap = h->len + tl;
        h = (logos_text_hdr *)lc_gc_realloc(h, sizeof(logos_text_hdr) + cap + 1);
        h->cap = cap;
        *s = (char *)(h + 1);
    }
    if (h->ascii) h->ascii = logos_text_is_ascii(text, tl);
    memcpy(*s + h->len, text, tl + 1);
    h->len += tl;
    lc_gc_free(inside);
}

/* Append an Int without allocating a temporary formatted string. Build the
   digits backwards so INT64_MIN is representable through its unsigned
   magnitude, exactly as LogosStrPushInt::push_i64 does in the Rust runtime. */
__attribute__((unused)) static void logos_text_append_i64(char **s, int64_t value) {
    char digits[21];
    size_t at = sizeof digits;
    uint64_t magnitude = value < 0 ? (uint64_t)0 - (uint64_t)value : (uint64_t)value;
    do {
        digits[--at] = (char)('0' + magnitude % 10);
        magnitude /= 10;
    } while (magnitude);
    if (value < 0) digits[--at] = '-';

    logos_text_hdr *h = (logos_text_hdr *)*s - 1;
    size_t len = sizeof digits - at;
    if (h->len + len > h->cap) {
        size_t cap = h->cap * 2;
        if (cap < h->len + len) cap = h->len + len;
        h = (logos_text_hdr *)lc_gc_realloc(h, sizeof(logos_text_hdr) + cap + 1);
        h->cap = cap;
        *s = (char *)(h + 1);
    }
    memcpy(*s + h->len, digits + at, len);
    h->len += len;
    (*s)[h->len] = '\0';
}

/* Append one-byte text literals without strlen or a generic text append.
   NUL is deliberately excluded by the emitter. */
__attribute__((unused)) static void logos_text_push_ascii(char **s, unsigned char byte) {
    logos_text_hdr *h = (logos_text_hdr *)*s - 1;
    if (h->len == h->cap) {
        size_t cap = h->cap ? h->cap * 2 : 1;
        h = (logos_text_hdr *)lc_gc_realloc(h, sizeof(logos_text_hdr) + cap + 1);
        h->cap = cap;
        *s = (char *)(h + 1);
    }
    (*s)[h->len++] = (char)byte;
    (*s)[h->len] = '\0';
}

/* The `n` texts `parts`, joined in one allocation; a part `owned` marks is freed once copied. */
__attribute__((unused)) static char *logos_text_join(size_t n, const char **parts, const unsigned char *owned) {
    size_t total = 0;
    for (size_t i = 0; i < n; i++) total += strlen(parts[i]);
    char *r = (char *)lc_gc_alloc(total + 1);
    size_t at = 0;
    for (size_t i = 0; i < n; i++) {
        size_t l = strlen(parts[i]);
        memcpy(r + at, parts[i], l);
        at += l;
        if (owned[i]) lc_gc_free((void *)parts[i]);
    }
    r[at] = '\0';
    return r;
}

/* The width in bytes of the UTF-8 character whose first byte is `b`. */
__attribute__((unused)) static size_t logos_utf8_width(unsigned char b) {
    return b < 0x80 ? 1 : b < 0xE0 ? 2 : b < 0xF0 ? 3 : 4;
}

/* `item k of` the text `s` of `len` bytes, `ascii` when every byte is below 0x80: its k-th character, counted
   from the end for a negative k, as a text of its own, as the VM's `index_get` reads it; an index naming no
   character fails the run as a list's does. A one-byte character's text is shared from a table of them rather
   than allocated per read. */
__attribute__((unused)) static char *logos_text_char_at_n(const char *s, size_t len, int ascii, int64_t k) {
    static char one[128][2];
    size_t at, width = 1;
    if (ascii) {
        at = logos_idx0(k, len);
    } else {
        size_t chars = 0;
        for (size_t i = 0; i < len; i += logos_utf8_width((unsigned char)s[i])) chars++;
        size_t nth = logos_idx0(k, chars);
        at = 0;
        for (size_t c = 0; c < nth; c++) at += logos_utf8_width((unsigned char)s[at]);
        width = logos_utf8_width((unsigned char)s[at]);
        if (width > len - at) width = len - at;
    }
    unsigned char b = (unsigned char)s[at];
    if (b < 0x80) {
        one[b][0] = (char)b;
        return one[b];
    }
    char *r = (char *)lc_gc_alloc(width + 1);
    memcpy(r, s + at, width);
    r[width] = '\0';
    return r;
}

/* `item k of` a builder. */
__attribute__((unused)) static char *logos_text_builder_char_at(const char *s, int64_t k) {
    const logos_text_hdr *h = (const logos_text_hdr *)s - 1;
    return logos_text_char_at_n(s, h->len, h->ascii, k);
}

/* `item k of` a plain text. */
__attribute__((unused)) static char *logos_text_char_at(const char *s, int64_t k) {
    size_t len = strlen(s);
    return logos_text_char_at_n(s, len, logos_text_is_ascii(s, len), k);
}
