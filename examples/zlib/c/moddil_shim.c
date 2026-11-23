/* The C side of Moddil's zlib bindings: zlib's API with fixed-width types.
 *
 * zlib's `uLong` is C's `unsigned long`, 32 bits on Windows and 64 elsewhere, and `z_stream`
 * holds `uLong`s, so its layout differs too. These wrappers take `uint64_t` lengths and keep a
 * `z_stream` behind an opaque pointer, so the Moddil declarations are the same on every
 * platform (the usual practice for a portable C ABI).
 */

#include <stdint.h>
#include <stdlib.h>

#include "zlib.h"

/* What a deflater has done: `Stats` in `src/lib.moddil`, `#[repr(C)]`. */
typedef struct {
    uint64_t total_in;
    uint64_t total_out;
    uint32_t adler;
} moddil_zlib_stats;

uint64_t moddil_zlib_bound(uint64_t len) {
    return (uint64_t)compressBound((uLong)len);
}

int32_t moddil_zlib_compress(uint8_t *dst, uint64_t *dst_len, const uint8_t *src,
                             uint64_t src_len, int32_t level) {
    uLongf n = (uLongf)*dst_len;
    int r = compress2(dst, &n, src, (uLong)src_len, level);
    *dst_len = (uint64_t)n;
    return r;
}

int32_t moddil_zlib_uncompress(uint8_t *dst, uint64_t *dst_len, const uint8_t *src,
                               uint64_t src_len) {
    uLongf n = (uLongf)*dst_len;
    uLong m = (uLong)src_len;
    int r = uncompress2(dst, &n, src, &m);
    *dst_len = (uint64_t)n;
    return r;
}

uint32_t moddil_zlib_crc32(uint32_t crc, const uint8_t *buf, uint64_t len) {
    return (uint32_t)crc32_z((uLong)crc, buf, (z_size_t)len);
}

uint32_t moddil_zlib_adler32(uint32_t adler, const uint8_t *buf, uint64_t len) {
    return (uint32_t)adler32_z((uLong)adler, buf, (z_size_t)len);
}

/* Streaming compression: a `z_stream` behind an opaque pointer. `*status` is zlib's code. */
void *moddil_zlib_deflater_new(int32_t level, int32_t *status) {
    z_stream *s = calloc(1, sizeof *s);
    if (s == NULL) {
        *status = Z_MEM_ERROR;
        return NULL;
    }
    *status = deflateInit(s, level);
    if (*status != Z_OK) {
        free(s);
        return NULL;
    }
    return s;
}

/* Compresses all `in_len` bytes at `in` (and, if `finish`, ends the stream) into the `out_cap`
 * bytes at `out`, which must have room for them (`moddil_zlib_bound`): how many it wrote, in
 * `*out_len`. zlib's code, or -100 if the output had no room left. */
int32_t moddil_zlib_deflate(void *h, const uint8_t *in, uint64_t in_len, uint8_t *out,
                            uint64_t out_cap, uint64_t *out_len, int32_t finish) {
    z_stream *s = h;
    s->next_in = (Bytef *)in;
    s->avail_in = (uInt)in_len;
    s->next_out = out;
    s->avail_out = (uInt)out_cap;
    int r = deflate(s, finish ? Z_FINISH : Z_NO_FLUSH);
    *out_len = out_cap - s->avail_out;
    if (s->avail_in != 0 || (finish && r != Z_STREAM_END)) {
        return -100;
    }
    return r;
}

void moddil_zlib_deflater_stats(void *h, moddil_zlib_stats *stats) {
    z_stream *s = h;
    stats->total_in = (uint64_t)s->total_in;
    stats->total_out = (uint64_t)s->total_out;
    stats->adler = (uint32_t)s->adler;
}

void moddil_zlib_deflater_end(void *h) {
    deflateEnd(h);
    free(h);
}
