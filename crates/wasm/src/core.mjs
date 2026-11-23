// moddil-wasm's core: what runs a Moddil program's module, in Node or in a browser. The test
// host (`host.cjs`) and the glue `moddil build --wasm` writes both use it.
//
// - Three modules: the program's, the runtime's (`moddil-rt` built for wasm32, sharing the
//   program's memory), and the shim (sharing the program's table, so the runtime calls the
//   program's task code and drop glue wasm to wasm).
// - Tasks (with JSPI): an entry, and each task, is a promising call. A task that hands over the
//   turn calls the runtime's import `moddil_host_switch`, which suspends it until its turn comes
//   again. Each task has its own two shadow stacks (the program's and the runtime's), swapped on
//   each switch, taken by growing memory and pooled.

/** Whether this engine has JSPI. */
export const jspi =
  typeof WebAssembly.Suspending === 'function' && typeof WebAssembly.promising === 'function';

const PSTACK = 4 << 20;
const RSTACK = 256 << 10;

/**
 * Instantiates a program.
 *
 * - `runtime`, `shim`, `program`: the modules' bytes (or compiled modules).
 * - `layout`: where the program's scratch, data and site tables are (`pages`, `table`, `out`,
 *   `err`, `record`, `words`, `dataFrom`, `dataTo`, `siteCount`, `sites`, `strings`), and
 *   whether it uses tasks (`tasks`).
 * - `observe(site, addr)`: a test's `// =>` observation, while the value is there to read.
 * - `log(level, text)`: an event of the ambient log.
 * - `promisingAlways`: run every entry as a promising call where there is JSPI (its stacks can
 *   grow further than a plain call's), not only those of a module with tasks.
 */
export async function instantiate({ runtime, shim, program, layout, observe, log, promisingAlways }) {
  const L = layout;
  const memory = new WebAssembly.Memory({ initial: L.pages });
  const table = new WebAssembly.Table({ element: 'anyfunc', initial: L.table });
  const bytes = (at, n) => {
    const size = memory.buffer.byteLength;
    const from = Math.min(at, size);
    const to = Math.min(at + n, size);
    const b = new Uint8Array(n);
    b.set(new Uint8Array(memory.buffer, from, to - from));
    return b;
  };
  const text = (at, n) => new TextDecoder().decode(bytes(at, n));
  const LEVELS = ['debug', 'info', 'warn', 'error'];
  const promising = jspi && (L.tasks || promisingAlways);

  let rt = null;
  let prog = null;
  let callTask = null;
  // The tasks of the run going on: each one's turn (a resolver, while it waits), its stacks'
  // pointers (while it waits), its stacks, and its call (a promise, once started).
  const pool = [];
  let tasks = new Map();
  let current = 0;
  let aborted = false;
  let crashed = null;
  const region = () => {
    if (pool.length > 0) return pool.pop();
    return memory.grow((PSTACK + RSTACK) / 65536) * 65536;
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
      const task = { turn: null, sp: 0, rsp: 0, call: null };
      tasks.set(t, task);
      current = t;
      prog.sp.value = base + PSTACK;
      rt.__stack_pointer.value = base + PSTACK + RSTACK;
      task.call = callTask(t).then(
        () => pool.push(base),
        (err) => {
          // The engine stopped the task: the run cannot go on.
          crashed = crashed || String(err);
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

  const host = {
    moddil_rt_observe: (site, addr) => observe(site, Number(addr)),
    moddil_rt_log: (level, p, n) => log(LEVELS[level], text(Number(p), Number(n))),
    moddil_host_switch: jspi
      ? new WebAssembly.Suspending(switchTo)
      : () => {
          throw new Error('tasks need JSPI');
        },
    moddil_host_resume: (next) => go(next),
    moddil_host_abort: () => abortAll(),
  };
  const shimI = await WebAssembly.instantiate(shim, { env: { table } });
  const rtI = await WebAssembly.instantiate(runtime, {
    env: { memory },
    shim: (shimI.instance ?? shimI).exports,
    host,
  });
  rt = (rtI.instance ?? rtI).exports;
  if (jspi) callTask = WebAssembly.promising(rt.moddil_wasm_task);
  const progI = await WebAssembly.instantiate(program, { env: { memory, table }, rt, host });
  prog = (progI.instance ?? progI).exports;
  const sp = prog.sp.value;
  const rsp = rt.__stack_pointer.value;
  const data = bytes(L.dataFrom, L.dataTo - L.dataFrom);

  // Calls `f` as a promising call (or a plain one), then waits for every task it left
  // suspended to be woken and to end.
  const call = async (f, args) => {
    tasks = new Map([[0, { turn: null, sp: 0, rsp: 0, call: null }]]);
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
  const suffix = (k) => (k === 0 ? '' : '_' + k);

  return {
    memory,
    bytes,
    /** Runs entry `k` afresh (the stacks, the data, a new run): its status. */
    run: async (k) => {
      prog.sp.value = sp;
      rt.__stack_pointer.value = rsp;
      new Uint8Array(memory.buffer).set(data, L.dataFrom);
      rt.moddil_wasm_begin();
      if (L.siteCount) rt.moddil_wasm_sites(L.siteCount, L.sites, L.strings);
      return call(prog['moddil_entry' + suffix(k)], [BigInt(L.out), BigInt(L.err)]);
    },
    /** Drops entry `k`'s result: its status. */
    drop: (k) => call(prog['moddil_drop_result' + suffix(k)], [BigInt(L.out)]),
    /** How many allocations are not freed. */
    live: () => Number(rt.moddil_wasm_live()),
    /** What the run recorded, as words (`moddil_wasm_record`). */
    record: () => {
      const n = rt.moddil_wasm_record(L.record, L.words);
      return Array.from(new Uint32Array(memory.buffer, L.record, n));
    },
    /** The exit code for `main`'s result (`main.exit.1`), for a built program. */
    exitCode: () => prog.moddil_exit_code(BigInt(L.out)),
    /** The run's trap, as its class and where it is reported (`file:line:column`). */
    trapText: () => {
      const n = rt.moddil_wasm_trap_text(L.record, L.words * 4);
      const s = text(L.record, n);
      const nl = s.indexOf('\n');
      return nl < 0 ? { class: s, at: '' } : { class: s.slice(0, nl), at: s.slice(nl + 1) };
    },
  };
}
