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

  // The OS a run of `main` has (`core::os`, `os.1`): Node's `net` module, where there is one.
  // Handles are numbers; what arrives is queued as events, which the runtime's scheduler polls
  // for when no task is ready, suspending (JSPI) until one comes or its next timer is due.
  const node = typeof process !== 'undefined' && !!process.versions?.node;
  const net = node ? await import('node:net') : null;
  const os = {
    args: [],
    handles: [],
    events: [],
    last: [],
    signals: false,
    wake: null,
  };
  const event = (kind, a, b) => {
    os.events.push([kind, a, b]);
    if (os.wake) {
      const w = os.wake;
      os.wake = null;
      w();
    }
  };
  const addConn = (socket) => {
    const h = { kind: 'conn', socket, buf: [], watched: false, ended: false };
    const id = os.handles.push(h) - 1;
    socket.on('data', (d) => {
      h.buf.push(Buffer.from(d));
      if (h.watched) event(1, id, d.length);
    });
    const end = () => {
      if (h.ended) return;
      h.ended = true;
      if (h.watched) event(2, id, 0);
    };
    socket.on('end', end);
    socket.on('close', end);
    socket.on('error', end);
    return id;
  };
  const argBytes = (i) => new TextEncoder().encode(os.args[i] ?? '');
  const hostOs = {
    moddil_host_os_arg_count: () => os.args.length,
    moddil_host_os_arg_len: (i) => argBytes(i).length,
    moddil_host_os_arg_copy: (i, out) => new Uint8Array(memory.buffer).set(argBytes(i), out),
    moddil_host_os_write_stderr: (p, n) => {
      if (node) process.stderr.write(text(p, n));
      else console.error(text(p, n));
    },
    moddil_host_os_random: () => Math.floor(Math.random() * 2 ** 52) + 1,
    moddil_host_os_now: () => performance.now(),
    moddil_host_os_wall_clock: () => Date.now() * 1e6,
    moddil_host_os_listen: (port) => {
      if (!net) return -1;
      const h = { kind: 'listener', server: null, watched: false, pending: [] };
      const id = os.handles.push(h) - 1;
      h.server = net.createServer((socket) => {
        const c = addConn(socket);
        if (h.watched) event(0, id, c);
        else h.pending.push(c);
      });
      h.server.listen(port, '127.0.0.1');
      return id;
    },
    moddil_host_os_connect: (p, n, port) => {
      if (!net) return -1;
      return addConn(net.connect(port, text(p, n)));
    },
    moddil_host_os_take: (conn, out, n) => {
      const h = os.handles[conn];
      if (!h || h.kind !== 'conn') return 0;
      const mem = new Uint8Array(memory.buffer);
      let k = 0;
      while (k < n && h.buf.length > 0) {
        const b = h.buf[0];
        const m = Math.min(n - k, b.length);
        mem.set(b.subarray(0, m), out + k);
        k += m;
        if (m === b.length) h.buf.shift();
        else h.buf[0] = b.subarray(m);
      }
      return k;
    },
    moddil_host_os_write: (conn, p, n) => {
      const h = os.handles[conn];
      if (!h || h.kind !== 'conn' || h.ended) return 0;
      h.socket.write(Buffer.from(bytes(p, n)));
      return 1;
    },
    moddil_host_os_close: (id) => {
      const h = os.handles[id];
      if (!h) return;
      if (h.kind === 'conn') h.socket.end();
      else if (h.kind === 'listener') h.server.close();
      os.handles[id] = { kind: 'closed' };
    },
    moddil_host_os_watch: (id) => {
      const h = os.handles[id];
      if (!h || h.watched) return;
      h.watched = true;
      if (h.kind === 'listener') {
        for (const c of h.pending) event(0, id, c);
        h.pending = [];
      } else if (h.kind === 'conn') {
        const n = h.buf.reduce((s, b) => s + b.length, 0);
        if (n > 0) event(1, id, n);
        if (h.ended) event(2, id, 0);
      }
    },
    moddil_host_os_watch_signals: () => {
      os.signals = true;
    },
    moddil_host_os_waiting: () =>
      os.signals ||
      os.handles.some((h) => (h.kind === 'listener' && h.watched) || (h.kind === 'conn' && h.watched && !h.ended))
        ? 1
        : 0,
    moddil_host_os_event: (i, out) => {
      const [kind, a, b] = os.last[i];
      const f = new Float64Array(memory.buffer, out, 2);
      f[0] = a;
      f[1] = b;
      return kind;
    },
  };
  const poll = async (ms) => {
    if (os.events.length === 0) {
      await new Promise((r) => {
        os.wake = r;
        if (ms >= 0) setTimeout(r, ms);
      });
      os.wake = null;
    }
    os.last = os.events.splice(0);
    return os.last.length;
  };
  const promising = jspi && (L.tasks || promisingAlways);

  // The GPU and the window a run of `main` has in a browser (`gpu.1`): WebGPU, a canvas the
  // image is shown on, and the page's keyboard and mouse. Without WebGPU a dispatch says it
  // could not (so `gpu::launch` and `gpu::present` run the kernel on the CPU simulator, and
  // `gpu::show` draws its image on a 2D canvas); outside a page, a frame asks the program to
  // stop. A page's `?size=WxH` fixes the window's size, and `?frames=N` stops the program after
  // N frames (as the browser test does).
  //
  // With WebGPU, an image is drawn into a pixel buffer and shown by a full-screen pass that reads
  // it, so a frame costs no copy back to the CPU; it is read back only when the program asks.
  // Pipelines, buffers and bind groups are made once and kept (buffers grow as images do). A
  // corner of the page shows the frame rate, the image's size and the GPU's time per frame.
  const page = typeof document !== 'undefined';
  const query = page ? new URLSearchParams(location.search) : null;
  const screen = {
    device: undefined,
    format: null,
    canvas: null,
    context: null,
    overlay: null,
    pipelines: new Map(),
    blit: null,
    buffers: {},
    groups: new Map(),
    keys: 0,
    dx: 0,
    dy: 0,
    wheel: 0,
    last: null,
    frames: 0,
    fps: 0,
    gpuMs: null,
    size: '',
    dispatches: 0,
  };
  const KEYS = { KeyW: 1, KeyA: 2, KeyS: 4, KeyD: 8, KeyQ: 16, KeyE: 32 };
  // The canvas and the overlay, made (with the input's listeners) the first time they are needed.
  const canvas = () => {
    if (screen.canvas) return screen.canvas;
    const c = document.createElement('canvas');
    c.id = 'screen';
    c.style.cssText = 'position:fixed;inset:0;width:100vw;height:100vh;z-index:-1;background:#000';
    document.body.prepend(c);
    const o = document.createElement('div');
    o.id = 'stats';
    o.style.cssText =
      'position:fixed;right:8px;top:8px;padding:4px 8px;font:12px ui-monospace,monospace;' +
      'color:#e8eef8;background:rgba(0,0,0,.45);border-radius:4px;pointer-events:none';
    document.body.append(o);
    screen.overlay = o;
    addEventListener('keydown', (e) => {
      screen.keys |= KEYS[e.code] ?? 0;
    });
    addEventListener('keyup', (e) => {
      screen.keys &= ~(KEYS[e.code] ?? 0);
    });
    c.addEventListener('click', () => c.requestPointerLock?.());
    addEventListener('mousemove', (e) => {
      if (document.pointerLockElement !== c && !(e.buttons & 1)) return;
      screen.dx += e.movementX;
      screen.dy += e.movementY;
    });
    addEventListener('wheel', (e) => {
      screen.wheel += Math.sign(e.deltaY);
    });
    screen.canvas = c;
    return c;
  };
  // The window's size: the page's `?size=WxH`, or the window's in CSS pixels. The program
  // draws a fraction of it, as fast frames allow.
  const windowSize = () => {
    const m = /^(\d+)x(\d+)$/.exec(query.get('size') ?? '');
    if (m) return [Number(m[1]), Number(m[2])];
    return [Math.max(8, innerWidth), Math.max(8, innerHeight)];
  };
  const device = async () => {
    if (screen.device !== undefined) return screen.device;
    screen.device = null;
    try {
      const adapter = await navigator.gpu?.requestAdapter();
      if (adapter) {
        screen.device = await adapter.requestDevice();
        screen.format = navigator.gpu.getPreferredCanvasFormat();
      }
    } catch {
      screen.device = null;
    }
    // For a test (and anyone curious): no WebGPU here, so the CPU simulator draws.
    if (!screen.device && page) document.body.dataset.gpu = 'none';
    return screen.device;
  };
  // Buffer `name`, at least `bytes` long, with `usage`: kept, and made again (bigger) only when
  // it is too small; a new one forgets the bind groups that named the old.
  const buffer = (d, name, bytes, usage) => {
    const size = Math.max(16, Math.ceil(bytes / 16) * 16);
    const b = screen.buffers[name];
    if (b && b.size >= size) return b;
    b?.destroy();
    screen.groups.clear();
    const made = d.createBuffer({ size, usage });
    screen.buffers[name] = made;
    return made;
  };
  // The bind group of `layout`'s `buffers`, kept by `key` until a buffer is made again.
  const group = (d, key, layout, buffers) => {
    let g = screen.groups.get(key);
    if (!g) {
      g = d.createBindGroup({
        layout,
        entries: buffers.map((b, i) => ({ binding: i, resource: { buffer: b } })),
      });
      screen.groups.set(key, g);
    }
    return g;
  };
  // The pass that shows the pixel buffer: a triangle covering the canvas, each fragment the
  // pixel under it (the canvas is the image's size; CSS stretches it to the window).
  const BLIT = `
@group(0) @binding(0) var<storage, read> pixels: array<u32>;
@group(0) @binding(1) var<storage, read> size: array<u32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
  var p = array(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  return vec4f(p[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4f) -> @location(0) vec4f {
  let x = min(u32(at.x), size[0] - 1u);
  let y = min(u32(at.y), size[1] - 1u);
  return unpack4x8unorm(pixels[y * size[0] + x]);
}`;
  // Encodes showing the pixel buffer (`width` x `height`) on the canvas.
  const encodeShow = (d, enc, width, height) => {
    const c = canvas();
    if (!screen.context) {
      screen.context = c.getContext('webgpu');
      screen.context.configure({ device: d, format: screen.format, alphaMode: 'opaque' });
    }
    if (c.width !== width || c.height !== height) {
      c.width = width;
      c.height = height;
    }
    if (!screen.blit) {
      const module = d.createShaderModule({ code: BLIT });
      screen.blit = d.createRenderPipeline({
        layout: 'auto',
        vertex: { module, entryPoint: 'vs' },
        fragment: { module, entryPoint: 'fs', targets: [{ format: screen.format }] },
        primitive: { topology: 'triangle-list' },
      });
    }
    const pass = enc.beginRenderPass({
      colorAttachments: [
        {
          view: screen.context.getCurrentTexture().createView(),
          loadOp: 'clear',
          storeOp: 'store',
          clearValue: { r: 0, g: 0, b: 0, a: 1 },
        },
      ],
    });
    pass.setPipeline(screen.blit);
    const { pixels, size } = screen.buffers;
    pass.setBindGroup(0, group(d, 'blit', screen.blit.getBindGroupLayout(0), [pixels, size]));
    pass.draw(3);
    pass.end();
  };
  const STORAGE = () => GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST;
  // The pixel and size buffers for a `width` x `height` image.
  const imageBuffers = (d, width, height) => {
    const pixels = buffer(d, 'pixels', width * height * 4, STORAGE() | GPUBufferUsage.COPY_SRC);
    const size = buffer(d, 'size', 16, STORAGE());
    d.queue.writeBuffer(size, 0, new Uint32Array([width, height, 0, 0]));
    return { pixels, size };
  };
  // Runs the kernel `source` over a `width` x `height` image: a workgroup of 8 x 8 invocations
  // per 8 x 8 tile. `mode` 1 shows the image, 2 reads its pixels back into `out`. 1 if it ran.
  const dispatch = async (sp, sn, pp, pn, width, height, op, on, mode) => {
    const d = page ? await device() : null;
    if (!d || width <= 0 || height <= 0) return 0;
    try {
      const source = text(sp >>> 0, sn >>> 0);
      let pipeline = screen.pipelines.get(source);
      if (!pipeline) {
        const module = d.createShaderModule({ code: source });
        pipeline = d.createComputePipeline({ layout: 'auto', compute: { module, entryPoint: 'main' } });
        screen.pipelines.set(source, pipeline);
      }
      const n = pn >>> 0;
      const params = buffer(d, 'params', n, STORAGE());
      const padded = new Uint8Array(Math.ceil(n / 4) * 4);
      padded.set(bytes(pp >>> 0, n));
      d.queue.writeBuffer(params, 0, padded);
      const { pixels, size } = imageBuffers(d, width, height);
      const enc = d.createCommandEncoder();
      const pass = enc.beginComputePass();
      pass.setPipeline(pipeline);
      pass.setBindGroup(0, group(d, source, pipeline.getBindGroupLayout(0), [params, pixels, size]));
      pass.dispatchWorkgroups(Math.ceil(width / 8), Math.ceil(height / 8));
      pass.end();
      if (mode & 1) encodeShow(d, enc, width, height);
      const words = Math.min(on >>> 0, width * height);
      let read = null;
      if (mode & 2) {
        read = buffer(d, 'read', words * 4, GPUBufferUsage.MAP_READ | GPUBufferUsage.COPY_DST);
        enc.copyBufferToBuffer(pixels, 0, read, 0, words * 4);
      }
      const t0 = performance.now();
      d.queue.submit([enc.finish()]);
      screen.size = `${width}x${height}`;
      // The GPU's time, now and then: how long until the queue is done (this frame waits).
      if (screen.dispatches++ % 30 === 0) {
        await d.queue.onSubmittedWorkDone();
        screen.gpuMs = performance.now() - t0;
      }
      if (read) {
        await read.mapAsync(GPUMapMode.READ, 0, words * 4);
        const got = new Uint32Array(read.getMappedRange(0, words * 4).slice(0));
        read.unmap();
        new Uint32Array(memory.buffer, op >>> 0, words).set(got);
      }
      // For a test (and anyone curious): the GPU drew a frame.
      document.body.dataset.gpu = 'used';
      return 1;
    } catch (e) {
      console.error('the GPU could not run the kernel:', e);
      return 0;
    }
  };
  // Shows `n` pixels at `p`, a `width` x `height` image the CPU drew: through WebGPU's pass
  // where the GPU is in use, else on a 2D canvas.
  const show = (p, n, width, height) => {
    if (!page || width <= 0 || height <= 0) return;
    const pixels = new Uint32Array(memory.buffer, p >>> 0, Math.min(n >>> 0, width * height)).slice();
    const d = screen.device;
    screen.size = `${width}x${height}`;
    if (d) {
      const b = imageBuffers(d, width, height);
      d.queue.writeBuffer(b.pixels, 0, pixels);
      const enc = d.createCommandEncoder();
      encodeShow(d, enc, width, height);
      d.queue.submit([enc.finish()]);
      return;
    }
    const c = canvas();
    if (c.width !== width || c.height !== height) {
      c.width = width;
      c.height = height;
    }
    const rgba = new Uint8ClampedArray(pixels.buffer, 0, width * height * 4);
    c.getContext('2d').putImageData(new ImageData(rgba, width, height), 0, 0);
  };
  // Waits for the next animation frame, then writes its 8 words to `out`: the microseconds
  // since the last frame, the window's size, the keys held, the mouse's and wheel's movement,
  // and whether to stop.
  const frame = async (out) => {
    const words = [0, 0, 0, 0, 0, 0, 0, 1];
    if (page) {
      canvas();
      const now = await new Promise((r) => requestAnimationFrame(r));
      const dt = screen.last === null ? 0 : Math.round((now - screen.last) * 1000);
      if (screen.last !== null && dt > 0) {
        const fps = 1e6 / dt;
        screen.fps = screen.fps ? screen.fps * 0.9 + fps * 0.1 : fps;
      }
      screen.last = now;
      screen.frames += 1;
      const gpu = screen.gpuMs === null ? '' : ` · gpu ${screen.gpuMs.toFixed(1)} ms`;
      screen.overlay.textContent = `${screen.fps.toFixed(0)} fps · ${screen.size}${gpu}`;
      const limit = Number(query.get('frames') ?? 0);
      const [w, h] = windowSize();
      words.splice(0, 8, dt, w, h, screen.keys, screen.dx, screen.dy, screen.wheel, 0);
      if (limit > 0 && screen.frames > limit) words[7] = 1;
      screen.dx = 0;
      screen.dy = 0;
      screen.wheel = 0;
    }
    new BigInt64Array(memory.buffer, out >>> 0, 8).set(words.map(BigInt));
  };

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
    ...hostOs,
    moddil_host_os_poll: jspi
      ? new WebAssembly.Suspending(poll)
      : () => {
          throw new Error('the OS needs JSPI');
        },
    moddil_host_gpu_dispatch: jspi ? new WebAssembly.Suspending(dispatch) : () => 0,
    moddil_host_gpu_show: show,
    moddil_host_gpu_frame: jspi
      ? new WebAssembly.Suspending(frame)
      : (out) => new BigInt64Array(memory.buffer, out >>> 0, 8).set([0n, 0n, 0n, 0n, 0n, 0n, 0n, 1n]),
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
  const call = async (f, args, alwaysPromising = false) => {
    tasks = new Map([[0, { turn: null, sp: 0, rsp: 0, call: null }]]);
    current = 0;
    aborted = false;
    crashed = null;
    const s =
      promising || (alwaysPromising && jspi) ? await WebAssembly.promising(f)(...args) : f(...args);
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
    /**
     * Runs entry `k`, `main`, with the OS (`os.1`): `args` are the program's arguments; its
     * status.
     */
    runMain: async (k, args) => {
      prog.sp.value = sp;
      rt.__stack_pointer.value = rsp;
      new Uint8Array(memory.buffer).set(data, L.dataFrom);
      rt.moddil_wasm_begin();
      if (L.siteCount) rt.moddil_wasm_sites(L.siteCount, L.sites, L.strings);
      os.args = args;
      rt.moddil_wasm_os();
      // A promising call, so the runtime can wait for the OS (`moddil_host_os_poll`).
      const s = await call(prog['moddil_entry' + suffix(k)], [BigInt(L.out), BigInt(L.err)], true);
      for (const h of os.handles) {
        if (h.kind === 'conn') h.socket.destroy();
        else if (h.kind === 'listener') h.server.close();
      }
      return s;
    },
    /** A signal for the run of `main`: 1 an interrupt, 2 a request to end (`os.1`). */
    signal: (n) => {
      if (os.signals) event(3, n, 0);
      return os.signals;
    },
    /** Drops entry `k`'s result: its status. */
    drop: (k) => call(prog['moddil_drop_result' + suffix(k)], [BigInt(L.out)]),
    /** How many allocations are not freed. */
    live: () => Number(rt.moddil_wasm_live()),
    /** Sets the run's hash seed (`coll.hash.2`), given as decimal text. */
    hashSeed: (s) => rt.moddil_wasm_hash_seed(BigInt(s)),
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
