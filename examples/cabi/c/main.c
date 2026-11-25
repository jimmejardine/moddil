/* C calling the Moddil library `cabi` through its generated header. Exits with 0 if every
 * result is right, else with the number of the first wrong one. */
#include <stdio.h>

#include "cabi.h"

int main(void) {
    Point a = {0.0, 0.0};
    Point b = {3.0, 4.0};
    if (moddil_gcd(84, -36) != 12) return 1;
    if (!moddil_is_even(4) || moddil_is_even(7)) return 2;
    if (moddil_distance(&a, &b) != 5.0) return 3;
    moddil_scale(&b, 2.0);
    if (b.x != 6.0 || b.y != 8.0) return 4;

    /* Structs by value, in each shape the C conventions treat differently. */
    Point m = moddil_midpoint(a, b);
    if (m.x != 3.0 || m.y != 4.0) return 5;
    Pair p = moddil_swap((Pair){1, 2});
    if (p.a != 2 || p.b != 1) return 6;
    Mixed x = moddil_bump((Mixed){1.0, 7});
    if (x.d != 1.5 || x.i != 8) return 7;
    Triple t = moddil_rotate((Triple){1, 2, 3});
    if (t.a != 2 || t.b != 3 || t.c != 1) return 8;
    Halves h = moddil_halves((Halves){1.5f, 2.5f});
    if (h.lo != 2.5f || h.hi != 1.5f) return 9;

    /* A trap gives the result type's zero, or the function's `on_trap`, and moddil_trapped()
     * says so until the next call. The library goes on working. */
    if (moddil_divide(7, 2) != 3 || moddil_trapped()) return 10;
    if (moddil_divide(7, 0) != 0 || !moddil_trapped()) return 11;
    if (moddil_third(1) != 20 || moddil_trapped()) return 12;
    if (moddil_third(5) != -1 || !moddil_trapped()) return 13;
    Point z = moddil_along(-1.0);
    if (z.x != 0.0 || z.y != 0.0 || !moddil_trapped()) return 14;
    if (moddil_gcd(6, 4) != 2 || moddil_trapped()) return 15;
    puts("C called Moddil: every result is right");
    return 0;
}
