/* ========== Files ========== */

/* The files a program reads, writes and mounts, as the native file system resolves them: a path names a file under
   the working directory, its leading `/`s and then its leading `../`s dropped; a write replaces the file whole,
   through a sibling of the same name with the extension `tmp`, after creating its directories; a read takes the
   whole file as Text. A failure is reported as the interpreter words it, "Read error: " or "Write error: " and then
   the file system's own account of the fault. */

#include <errno.h>
#include <sys/stat.h>

__attribute__((unused)) static char *logos_file_path(const char *path) {
    while (*path == '/') path++;
    while (strncmp(path, "../", 3) == 0) path += 3;
    size_t n = strlen(path);
    char *out = (char *)lc_gc_alloc(n + 3);
    memcpy(out, "./", 2);
    memcpy(out + 2, path, n + 1);
    return out;
}

/* The account of an operating-system fault the native file system gives: its kind, then the error's own text. */
__attribute__((unused, noreturn)) static void logos_file_fail(const char *verb, int code) {
    const char *kind = code == ENOENT ? "Not found"
        : (code == EACCES || code == EPERM) ? "Permission denied"
        : code == EEXIST ? "Already exists"
        : "IO error";
    logos_fatalf("%s error: %s: %s (os error %d)", verb, kind, strerror(code), code);
}

__attribute__((unused)) static int logos_utf8_valid(const unsigned char *s, size_t n) {
    size_t i = 0;
    while (i < n) {
        unsigned char c = s[i];
        size_t len;
        uint32_t cp;
        if (c < 0x80) { i++; continue; }
        else if ((c & 0xE0) == 0xC0) { len = 2; cp = c & 0x1F; }
        else if ((c & 0xF0) == 0xE0) { len = 3; cp = c & 0x0F; }
        else if ((c & 0xF8) == 0xF0) { len = 4; cp = c & 0x07; }
        else return 0;
        if (i + len > n) return 0;
        for (size_t k = 1; k < len; k++) {
            if ((s[i + k] & 0xC0) != 0x80) return 0;
            cp = (cp << 6) | (s[i + k] & 0x3F);
        }
        if ((len == 2 && cp < 0x80) || (len == 3 && cp < 0x800) || (len == 4 && (cp < 0x10000 || cp > 0x10FFFF))
            || (cp >= 0xD800 && cp <= 0xDFFF)) return 0;
        i += len;
    }
    return 1;
}

/* The whole file at `path` as Text, or NULL with `errno` saying why (EILSEQ: not UTF-8). */
__attribute__((unused)) static char *logos_file_slurp(const char *path) {
    char *full = logos_file_path(path);
    FILE *f = fopen(full, "rb");
    lc_gc_free(full);
    if (!f) return NULL;
    size_t cap = 4096, len = 0;
    char *buf = (char *)lc_gc_alloc(cap);
    size_t got;
    while ((got = fread(buf + len, 1, cap - len - 1, f)) > 0) {
        len += got;
        if (cap - len - 1 == 0) { cap *= 2; buf = (char *)lc_gc_realloc(buf, cap); }
    }
    int failed = ferror(f);
    int saved = errno;
    fclose(f);
    if (failed) { lc_gc_free(buf); errno = saved; return NULL; }
    buf[len] = '\0';
    if (!logos_utf8_valid((const unsigned char *)buf, len)) { lc_gc_free(buf); errno = EILSEQ; return NULL; }
    return buf;
}

__attribute__((unused)) static char *logos_file_read(const char *path) {
    char *text = logos_file_slurp(path);
    if (text) return text;
    if (errno == EILSEQ) logos_fatal("Read error: IO error: stream did not contain valid UTF-8");
    logos_file_fail("Read", errno);
}

/* `Mount v at "path"`: the journal's text, or empty Text when there is none yet. */
__attribute__((unused)) static char *logos_file_mount(const char *path) {
    char *text = logos_file_slurp(path);
    return text ? text : lc_gc_strdup("");
}

__attribute__((unused)) static void logos_file_write(const char *path, const char *content) {
    char *full = logos_file_path(path);
    for (char *p = full + 2; *p; p++) {
        if (*p != '/') continue;
        *p = '\0';
        if (mkdir(full, 0777) != 0 && errno != EEXIST) { int code = errno; *p = '/'; logos_file_fail("Write", code); }
        *p = '/';
    }
    size_t n = strlen(full);
    const char *name = strrchr(full, '/') + 1;
    const char *dot = strrchr(name, '.');
    size_t stem = (dot && dot != name) ? (size_t)(dot - full) : n;
    char *temp = (char *)lc_gc_alloc(stem + 5);
    memcpy(temp, full, stem);
    memcpy(temp + stem, ".tmp", 5);
    FILE *f = fopen(temp, "wb");
    if (!f) logos_file_fail("Write", errno);
    size_t len = strlen(content);
    if (fwrite(content, 1, len, f) != len) { int code = errno; fclose(f); logos_file_fail("Write", code); }
    if (fclose(f) != 0) logos_file_fail("Write", errno);
    if (rename(temp, full) != 0) logos_file_fail("Write", errno);
    lc_gc_free(temp);
    lc_gc_free(full);
}
