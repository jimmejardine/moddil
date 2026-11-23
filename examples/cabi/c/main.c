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
    puts("C called Moddil: every result is right");
    return 0;
}
