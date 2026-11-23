// moddil-wasm's host: runs a module's entries in Node and reports, a JSON object a line:
//   {"o": site, "a": addr, "base": b, "hex": "..."}   an observation: the value's address, and
//                                                     the shadow stack in use, from `b`
//   {"e": name, "s": status, "t": site, ...}          an entry that ended: its status, the
//                                                     first trap's site (-1 for none), and, for
//                                                     `main`, its result's bytes
//   {"e": name, "crash": "..."}                       an entry the engine stopped
// Usage: node host.cjs module.wasm <stack top> <out> <entry>[:<result size>]...
'use strict';
const fs = require('fs');

const [, , path, top, outAt, ...entries] = process.argv;
const STACK_TOP = Number(top);
const OUT = Number(outAt);
const say = (o) => process.stdout.write(JSON.stringify(o) + '\n');

(async () => {
  let instance = null;
  let trapSite = -1;
  const hex = (at, n) =>
    Buffer.from(new Uint8Array(instance.exports.memory.buffer, at, n)).toString('hex');
  const env = {
    trap: (site) => {
      if (trapSite < 0) trapSite = site;
    },
    observe: (site, addr) => {
      const base = instance.exports.sp.value;
      say({ o: site, a: addr, base, hex: hex(base, STACK_TOP - base) });
    },
    // JavaScript's `%` on numbers is IEEE 754's remainder, as `libm`'s `fmod`.
    fmod: (a, b) => a % b,
    fmodf: (a, b) => Math.fround(a % b),
  };
  ({ instance } = await WebAssembly.instantiate(fs.readFileSync(path), { env }));
  for (const entry of entries) {
    const [name, size] = entry.split(':');
    instance.exports.sp.value = STACK_TOP;
    instance.exports.depth.value = 0;
    trapSite = -1;
    let status;
    try {
      status = name === 'main' ? instance.exports.main(OUT) : instance.exports[name]();
    } catch (err) {
      say({ e: name, crash: String(err) });
      continue;
    }
    const done = { e: name, s: status, t: trapSite };
    if (name === 'main' && status === 0) {
      done.base = OUT;
      done.hex = hex(OUT, Number(size));
    }
    say(done);
  }
})().catch((err) => {
  say({ fatal: String(err) });
  process.exit(1);
});
