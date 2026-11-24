"""Checks the Moddil module `stats` from Python: each function, buffers crossing without a copy,
the exceptions, and the class. Run with the module importable (a wheel installed, or beside it)."""

import array

import stats

xs = array.array("d", [1.0, 2.0, 3.0, 4.0])
assert stats.mean(xs) == 2.5
assert stats.mean(memoryview(xs)) == 2.5
assert stats.mean(array.array("d")) == 0.0
r = stats.rolling_mean(xs, 2)
assert isinstance(r, stats.Array) and list(r) == [1.5, 2.5, 3.5] and len(r) == 3
assert r[1] == 2.5 and r.tolist() == [1.5, 2.5, 3.5] and repr(r) == "Array([1.5, 2.5, 3.5])"
# An Array is a buffer over the `Vec`, without a copy: read-only, of the element's format.
m = memoryview(r)
assert m.format == "d" and m.readonly and m.tolist() == [1.5, 2.5, 3.5]
assert stats.mean(r) == 2.5
try:
    r[3]
    raise AssertionError("an index past the end is an IndexError")
except IndexError:
    pass
assert list(stats.steps(10, 3, -4)) == [10, 6, 2] and memoryview(stats.steps(0, 2, 1)).format == "q"
assert len(stats.steps(0, 0, 1)) == 0

# By keyword too.
assert list(stats.rolling_mean(xs, window=2)) == [1.5, 2.5, 3.5]
assert list(stats.steps(n=2, step=5, start=1)) == [1, 6]

try:
    stats.rolling_mean(xs, 9)
    raise AssertionError("a window wider than the values is an error")
except stats.StatsError as e:
    assert isinstance(e, stats.Error)
    assert "a window of 9 over 4 values" in str(e)
    # The error's fields are the exception's attributes.
    assert e.window == 9 and e.reason == "a window of 9 over 4 values"

# A `&mut [f64]` changes the caller's buffer: no copy.
stats.scale_in_place(xs, 10.0)
assert list(xs) == [10.0, 20.0, 30.0, 40.0]

assert stats.greet("Python") == "hello, Python, from Moddil"
assert stats.greet("Größe 😀") == "hello, Größe 😀, from Moddil"
assert stats.checksum(b"abcde") == stats.checksum(bytearray(b"abcde"))
assert stats.parse("123") == 123
assert stats.parse("12x") is None

try:
    stats.divide(1, 0)
    raise AssertionError("dividing by zero traps")
except stats.Trap:
    pass
assert stats.divide(7, 2) == 3  # the module still works after a trap

c = stats.Counter(5)
c.add(2)
assert c.count == 7 and c.doubled() == 14 and c.label == "counter"
c.count = 40
assert c.count == 40
f = c.fork("other")
assert (f.count, f.label) == (40, "other")
assert stats.Counter.zero().count == 0
assert stats.Counter(start=3).count == 3
for i in range(10000):
    stats.Counter(i).add(1)

# A method taking `self` by value takes the instance's value: it has none after.
g = stats.Counter(2)
assert g.finish() == "counter at 2"
try:
    g.count
    raise AssertionError("a counter given up has no value")
except ValueError:
    pass

# A submodule: `stats.geo`, importable by its dotted name too.
import stats.geo
from stats.geo import Rect

assert stats.geo.area(2.0, 3.0) == 6.0 and stats.geo.area(h=1.0, w=4.0) == 4.0
t = Rect(2.0, 5.0).turned()
assert (t.w, t.h) == (5.0, 2.0) and type(t).__module__ == "stats.geo"

for bad, call in [
    (TypeError, lambda: stats.mean(array.array("i", [1, 2]))),
    (TypeError, lambda: stats.checksum("text")),
    (OverflowError, lambda: stats.divide(2**70, 1)),
    (TypeError, lambda: stats.Counter("five")),
]:
    try:
        call()
        raise AssertionError(f"{bad.__name__} expected")
    except bad:
        pass

# numpy arrays are buffers too, where numpy is installed.
try:
    import numpy as np
except ImportError:
    # On CI (GitHub sets `CI`) numpy is installed, and its checks must run.
    import os

    if os.environ.get("CI"):
        raise
    np = None
if np is not None:
    a = np.array([1.0, 2.0, 3.0, 4.0])
    assert stats.mean(a) == 2.5
    stats.scale_in_place(a, 3.0)
    assert a.tolist() == [3.0, 6.0, 9.0, 12.0]
    assert stats.mean(np.arange(1_000_000, dtype=np.float64)) == 499999.5
    for bad in (np.array([1, 2], dtype=np.int32), a[::2]):
        try:
            stats.mean(bad)
            raise AssertionError("a wrong or strided array is refused")
        except (TypeError, ValueError):
            pass

print("stats: every check passed")
