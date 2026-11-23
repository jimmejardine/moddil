// moddil-wasm's test host: runs a program's module with the shared core (`core.mjs`, beside
// it), in a worker thread whose stack is deep enough for the calls a run may make, as the Rust
// side says. It reads a command a line from standard input and answers a JSON object a line:
//   run k          entry k's trampoline: {"s": status}, or {"crash": why}
//   drop k         entry k's drop of its result: {"s": status}, or {"crash": why}
//   live           how many allocations are not freed: {"n": n}
//   record         what the run recorded: {"w": "word,word,..."}
//   read a n       n bytes of memory from a: {"hex": "..."}
//   go             (while an observation waits) the run goes on
//   quit           the host ends
// An observation, while a run goes on, is {"o": site, "a": address}; the host then answers
// commands (reads) until `go`. It starts with {"ready": 1, "jspi": 0 or 1}, or {"fatal": why}.
//
// Usage: node host.cjs runtime.wasm shim.wasm module.wasm pages table out err record words
//   data_from data_to site_count sites strings tasks
'use strict';
const fs = require('fs');
const path = require('path');
const { pathToFileURL } = require('url');
const { Worker, isMainThread, workerData } = require('worker_threads');

if (isMainThread) {
  // A task's stack (JSPI's, each promising call's) as deep as a run's calls may go: fixed at
  // about 1 MB, it would overflow before the interpreters' limit. Growable, it grows up to the
  // stack size (in KiB), within the worker's. Set before the worker's isolate exists; a Node
  // without these flags ignores them.
  const v8 = require('v8');
  v8.setFlagsFromString('--experimental-wasm-growable-stacks');
  v8.setFlagsFromString('--stack-size=200000');
  const w = new Worker(__filename, {
    workerData: process.argv.slice(2),
    resourceLimits: { stackSizeMb: 256 },
    stdin: false,
    stdout: false,
    stderr: false,
  });
  w.on('error', (err) => {
    fs.writeSync(1, JSON.stringify({ fatal: String(err) }) + '\n');
    process.exit(1);
  });
  w.on('exit', (code) => process.exit(code));
} else {
  const say = (o) => fs.writeSync(1, JSON.stringify(o) + '\n');

  // Standard input, a line at a time.
  let pending = '';
  const buf = Buffer.alloc(1 << 16);
  const line = () => {
    for (;;) {
      const nl = pending.indexOf('\n');
      if (nl >= 0) {
        const l = pending.slice(0, nl).trim();
        pending = pending.slice(nl + 1);
        return l;
      }
      let n;
      try {
        n = fs.readSync(0, buf, 0, buf.length, null);
      } catch (e) {
        if (e.code === 'EAGAIN') continue;
        if (e.code === 'EOF') return 'quit';
        throw e;
      }
      if (n === 0) return 'quit';
      pending += buf.toString('utf8', 0, n);
    }
  };

  (async () => {
    const [rtPath, shimPath, modPath, ...nums] = workerData;
    const [pages, table, out, err, record, words, dataFrom, dataTo, siteCount, sites, strings, tasks] =
      nums.map(Number);
    const core = await import(pathToFileURL(path.join(__dirname, 'core.mjs')).href);
    let p = null;
    const hex = (a, n) => Buffer.from(p.bytes(Number(a), Number(n))).toString('hex');
    // Answers reads while an observation waits, until `go`.
    const observe = (site, addr) => {
      say({ o: site, a: addr });
      for (;;) {
        const [cmd, a, b] = line().split(' ');
        if (cmd === 'go') return;
        if (cmd === 'quit') process.exit(0);
        if (cmd === 'read') say({ hex: hex(a, b) });
        else say({ fatal: 'an unknown command while observing: ' + cmd });
      }
    };
    const log = (level, text) => fs.writeSync(2, `[${level}] ${text}\n`);
    try {
      p = await core.instantiate({
        runtime: fs.readFileSync(rtPath),
        shim: fs.readFileSync(shimPath),
        program: fs.readFileSync(modPath),
        layout: {
          pages,
          table,
          out,
          err,
          record,
          words,
          dataFrom,
          dataTo,
          siteCount,
          sites,
          strings,
          tasks: tasks === 1,
        },
        observe,
        log,
      });
    } catch (e) {
      say({ fatal: String(e) });
      process.exit(1);
    }
    say({ ready: 1, jspi: core.jspi ? 1 : 0 });
    for (;;) {
      const [cmd, a, b] = line().split(' ');
      if (cmd === 'quit') process.exit(0);
      if (cmd === 'read') {
        say({ hex: hex(a, b) });
      } else if (cmd === 'live') {
        say({ n: p.live() });
      } else if (cmd === 'record') {
        say({ w: p.record().join(',') });
      } else if (cmd === 'run' || cmd === 'drop') {
        try {
          const s = cmd === 'run' ? await p.run(Number(a)) : await p.drop(Number(a));
          say({ s });
        } catch (e) {
          say({ crash: String(e) });
        }
      } else {
        say({ fatal: 'an unknown command: ' + cmd });
      }
    }
  })().catch((e) => {
    say({ fatal: String(e) });
    process.exit(1);
  });
}
