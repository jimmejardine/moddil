// moddil-wasm's host: runs a program's module, with the runtime module sharing its memory, in a
// worker thread (whose stack is deep enough for the calls a run may make), as the Rust side
// says. It reads a command a line from standard input and answers a JSON object a line:
//   run k          entry k's trampoline: {"s": status}, or {"crash": why}
//   drop k         entry k's drop of its result: {"s": status}, or {"crash": why}
//   live           how many allocations are not freed: {"n": n}
//   record         what the run recorded: {"w": "word,word,..."}
//   read a n       n bytes of memory from a: {"hex": "..."}
//   go             (while an observation waits) the run goes on
//   quit           the host ends
// An observation, while a run goes on, is {"o": site, "a": address}; the host then answers
// commands (reads) until `go`. It starts with {"ready": 1}, or {"fatal": why}.
// Usage: node host.cjs runtime.wasm module.wasm pages out err record words data_from data_to
'use strict';
const fs = require('fs');
const { Worker, isMainThread, workerData } = require('worker_threads');

if (isMainThread) {
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
  const [rtPath, modPath, pages, out, err, record, words, dataFrom, dataTo] = workerData;
  const say = (o) => fs.writeSync(1, JSON.stringify(o) + '\n');
  const LEVELS = ['debug', 'info', 'warn', 'error'];

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

  const memory = new WebAssembly.Memory({ initial: Number(pages) });
  const bytes = (at, n) => {
    const size = memory.buffer.byteLength;
    const from = Math.min(at, size);
    const to = Math.min(at + n, size);
    const b = Buffer.alloc(n);
    Buffer.from(memory.buffer, from, to - from).copy(b);
    return b;
  };

  let rt = null;
  let prog = null;
  let sp = 0;
  let data = null;

  // Answers commands; `run` and `drop` call into the module; returns at `go` or `quit`.
  const serve = (inRun) => {
    for (;;) {
      const [cmd, a, b] = line().split(' ');
      if (cmd === 'go' && inRun) return;
      if (cmd === 'quit') process.exit(0);
      if (cmd === 'read') {
        say({ hex: bytes(Number(a), Number(b)).toString('hex') });
      } else if (cmd === 'live') {
        say({ n: Number(rt.moddil_wasm_live()) });
      } else if (cmd === 'record') {
        const n = rt.moddil_wasm_record(Number(record), Number(words));
        const w = new Uint32Array(memory.buffer, Number(record), n);
        say({ w: Array.from(w).join(',') });
      } else if ((cmd === 'run' || cmd === 'drop') && !inRun) {
        const k = Number(a);
        const suffix = k === 0 ? '' : '_' + k;
        try {
          let s;
          if (cmd === 'run') {
            // A fresh start: the stack, the data as it was, and a new run in the runtime.
            prog.sp.value = sp;
            new Uint8Array(memory.buffer).set(data, Number(dataFrom));
            rt.moddil_wasm_begin();
            s = prog['moddil_entry' + suffix](BigInt(out), BigInt(err));
          } else {
            s = prog['moddil_drop_result' + suffix](BigInt(out));
          }
          say({ s });
        } catch (e) {
          say({ crash: String(e) });
        }
      } else {
        say({ fatal: 'an unknown command: ' + cmd });
      }
    }
  };

  const host = {
    moddil_rt_observe: (site, addr) => {
      say({ o: site, a: Number(addr) });
      serve(true);
    },
    moddil_rt_log: (level, p, n) => {
      const text = bytes(Number(p), Number(n)).toString('utf8');
      fs.writeSync(2, `[${LEVELS[level]}] ${text}\n`);
    },
  };

  try {
    const r = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(rtPath)), {
      env: { memory },
    });
    rt = r.exports;
    const p = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(modPath)), {
      env: { memory },
      rt,
      host,
    });
    prog = p.exports;
    sp = prog.sp.value;
    data = bytes(Number(dataFrom), Number(dataTo) - Number(dataFrom));
  } catch (e) {
    say({ fatal: String(e) });
    process.exit(1);
  }
  say({ ready: 1 });
  serve(false);
}
