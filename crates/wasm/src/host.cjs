// moddil-wasm's host: runs a program's module in a worker thread (whose stack is deep enough
// for the calls a run may make), with the runtime module sharing its memory and the shim module
// its table, as the Rust side says. It reads a command a line from standard input and answers a
// JSON object a line:
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
// Tasks (with JSPI): each entry, and each task, is a promising call. A task that hands over the
// turn calls `moddil_host_switch`, which suspends it until its turn comes again. Each task has
// its own two shadow stacks (the program's and the runtime's), swapped on each switch.
//
// A module without tasks runs as plain calls, on the worker's own stack.
//
// Usage: node host.cjs runtime.wasm shim.wasm module.wasm pages table out err record words
//   data_from data_to site_count sites strings tasks
'use strict';
const fs = require('fs');
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
  const [rtPath, shimPath, modPath, ...nums] = workerData;
  const [
    pages,
    tableSize,
    out,
    err,
    record,
    words,
    dataFrom,
    dataTo,
    siteCount,
    sites,
    strings,
    usesTasks,
  ] = nums.map(Number);
  const say = (o) => fs.writeSync(1, JSON.stringify(o) + '\n');
  const LEVELS = ['debug', 'info', 'warn', 'error'];
  const jspi =
    typeof WebAssembly.Suspending === 'function' && typeof WebAssembly.promising === 'function';
  // Promising calls only for a module whose tasks suspend.
  const promising = jspi && usesTasks === 1;

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

  const memory = new WebAssembly.Memory({ initial: pages });
  const table = new WebAssembly.Table({ element: 'anyfunc', initial: tableSize });
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
  let rsp = 0;
  let data = null;

  // The tasks of the run going on: each one's turn (a resolver, while it waits), its stacks'
  // pointers (while it waits), its stacks, and its call (a promise, once started).
  const PSTACK = 4 << 20;
  const RSTACK = 256 << 10;
  const pool = [];
  let tasks = new Map();
  let current = 0;
  let aborted = false;
  let crashed = null;
  const region = () => {
    if (pool.length > 0) return pool.pop();
    return memory.grow((PSTACK + RSTACK) / 65536) * 65536;
  };
  const taskCall = jspi ? (rtExports) => WebAssembly.promising(rtExports.moddil_wasm_task) : null;
  let callTask = null;
  // Task `t` goes on: resumed if it waits, started if it has not run.
  const go = (t) =>
    queueMicrotask(() => {
      const e = tasks.get(t);
      if (e) {
        if (e.turn) {
          const r = e.turn;
          e.turn = null;
          r(aborted ? 1 : 0);
        }
        return;
      }
      const base = region();
      const task = { turn: null, base, sp: 0, rsp: 0, call: null };
      tasks.set(t, task);
      current = t;
      prog.sp.value = base + PSTACK;
      rt.__stack_pointer.value = base + PSTACK + RSTACK;
      task.call = callTask(t).then(
        () => pool.push(base),
        (e) => {
          // The engine stopped the task: the run cannot go on.
          crashed = crashed || String(e);
          abortAll();
        },
      );
    });
  const switchTo = async (next) => {
    const me = current;
    const e = tasks.get(me);
    e.sp = prog.sp.value;
    e.rsp = rt.__stack_pointer.value;
    const turn = new Promise((r) => {
      e.turn = r;
    });
    go(next);
    const r = await turn;
    current = me;
    prog.sp.value = e.sp;
    rt.__stack_pointer.value = e.rsp;
    return aborted ? 1 : r;
  };
  const abortAll = () => {
    aborted = true;
    for (const e of tasks.values()) {
      if (e.turn) {
        const r = e.turn;
        e.turn = null;
        queueMicrotask(() => r(1));
      }
    }
  };

  // Answers reads while an observation waits, until `go`.
  const serveObservation = () => {
    for (;;) {
      const [cmd, a, b] = line().split(' ');
      if (cmd === 'go') return;
      if (cmd === 'quit') process.exit(0);
      if (cmd === 'read') {
        say({ hex: bytes(Number(a), Number(b)).toString('hex') });
      } else {
        say({ fatal: 'an unknown command while observing: ' + cmd });
      }
    }
  };

  // Calls `f` as a promising call (or a plain one, without JSPI), then waits for every task it
  // left suspended to be woken and to end.
  const runCall = async (f, args) => {
    tasks = new Map([[0, { turn: null, base: null, sp: 0, rsp: 0, call: null }]]);
    current = 0;
    aborted = false;
    crashed = null;
    const s = promising ? await WebAssembly.promising(f)(...args) : f(...args);
    const left = [...tasks.values()].filter((e) => e.call);
    if (left.some((e) => e.turn)) abortAll();
    await Promise.all(left.map((e) => e.call));
    if (crashed) throw new Error(crashed);
    return s;
  };

  const serve = async () => {
    for (;;) {
      const [cmd, a, b] = line().split(' ');
      if (cmd === 'quit') process.exit(0);
      if (cmd === 'read') {
        say({ hex: bytes(Number(a), Number(b)).toString('hex') });
      } else if (cmd === 'live') {
        say({ n: Number(rt.moddil_wasm_live()) });
      } else if (cmd === 'record') {
        const n = rt.moddil_wasm_record(record, words);
        const w = new Uint32Array(memory.buffer, record, n);
        say({ w: Array.from(w).join(',') });
      } else if (cmd === 'run' || cmd === 'drop') {
        const k = Number(a);
        const suffix = k === 0 ? '' : '_' + k;
        try {
          let s;
          if (cmd === 'run') {
            // A fresh start: the stacks, the data as it was, and a new run in the runtime.
            prog.sp.value = sp;
            rt.__stack_pointer.value = rsp;
            new Uint8Array(memory.buffer).set(data, dataFrom);
            rt.moddil_wasm_begin();
            if (siteCount) rt.moddil_wasm_sites(siteCount, sites, strings);
            s = await runCall(prog['moddil_entry' + suffix], [BigInt(out), BigInt(err)]);
          } else {
            s = await runCall(prog['moddil_drop_result' + suffix], [BigInt(out)]);
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
      serveObservation();
    },
    moddil_rt_log: (level, p, n) => {
      const text = bytes(Number(p), Number(n)).toString('utf8');
      fs.writeSync(2, `[${LEVELS[level]}] ${text}\n`);
    },
    moddil_host_switch: jspi
      ? new WebAssembly.Suspending(switchTo)
      : () => {
          throw new Error('tasks need JSPI');
        },
    moddil_host_resume: (next) => go(next),
    moddil_host_abort: () => abortAll(),
  };

  try {
    const shim = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(shimPath)), {
      env: { table },
    });
    const r = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(rtPath)), {
      env: { memory },
      shim: shim.exports,
      host,
    });
    rt = r.exports;
    if (jspi) callTask = taskCall(rt);
    const p = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(modPath)), {
      env: { memory, table },
      rt,
      host,
    });
    prog = p.exports;
    sp = prog.sp.value;
    rsp = rt.__stack_pointer.value;
    data = bytes(dataFrom, dataTo - dataFrom);
  } catch (e) {
    say({ fatal: String(e) });
    process.exit(1);
  }
  say({ ready: 1, jspi: jspi ? 1 : 0 });
  serve().catch((e) => {
    say({ fatal: String(e) });
    process.exit(1);
  });
}
