/* C functions the Moddil library `cabi` calls, compiled with it (its manifest's `[c]`): structs
 * by value in each shape the C conventions treat differently, and functions that call back
 * through a function pointer Moddil gives. */
#include <stdint.h>

/* 16 bytes of floats: SSE registers (SysV), float registers (AArch64), a pointer (Windows). */
typedef struct {
    double x, y;
} Point;

/* 8 bytes of integers: one register everywhere. */
typedef struct {
    int32_t a, b;
} Pair;

/* A float and an integer: one of each register (SysV), two words (AArch64). */
typedef struct {
    double d;
    int32_t i;
} Mixed;

/* 24 bytes: in memory (SysV), through a pointer (Windows, AArch64). */
typedef struct {
    int64_t a, b, c;
} Triple;

/* Two floats in 8 bytes: one SSE register (SysV), two float registers (AArch64), one integer
 * register (Windows). */
typedef struct {
    float lo, hi;
} Halves;

Point shape_midpoint(Point a, Point b) {
    Point m = {(a.x + b.x) / 2.0, (a.y + b.y) / 2.0};
    return m;
}

Pair shape_swap(Pair p) {
    Pair s = {p.b, p.a};
    return s;
}

Mixed shape_bump(Mixed m) {
    Mixed r = {m.d + 0.5, m.i + 1};
    return r;
}

Triple shape_rotate(Triple t) {
    Triple r = {t.b, t.c, t.a};
    return r;
}

Halves shape_halves(Halves h) {
    Halves r = {h.hi, h.lo};
    return r;
}

/* Folds `xs` with `f`, from `init`. */
int64_t shape_fold(const int64_t *xs, int64_t n, int64_t (*f)(int64_t, int64_t), int64_t init) {
    int64_t acc = init;
    for (int64_t k = 0; k < n; k++) {
        acc = f(acc, xs[k]);
    }
    return acc;
}

/* Calls `f` on each of 1..=n with the caller's `data`, as C APIs with callbacks do. */
void shape_each(int32_t n, void (*f)(int32_t, void *), void *data) {
    for (int32_t k = 1; k <= n; k++) {
        f(k, data);
    }
}

static int64_t shape_mul(int64_t a, int64_t b) {
    return a * b;
}

static int64_t shape_sub(int64_t a, int64_t b) {
    return a - b;
}

/* A function pointer C gives back: multiplication, or subtraction for 0. */
int64_t (*shape_op(int32_t which))(int64_t, int64_t) {
    return which ? shape_mul : shape_sub;
}
