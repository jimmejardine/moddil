//! The OS a run of `main` has (`os.1`): `moddil-os` natively, and the host's functions (Node's
//! `net` module, through JSPI) in WebAssembly. Both give [`Event`]s for the scheduler to put
//! into channels.

pub(crate) use moddil_sched::Event;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::Event;

    /// The OS: `moddil-os`'s.
    pub(crate) struct Os(moddil_os::Os);

    impl Os {
        pub(crate) fn new(args: Vec<String>) -> Os {
            Os(moddil_os::Os::new(args))
        }

        pub(crate) fn arg_count(&self) -> usize {
            self.0.args().len()
        }

        pub(crate) fn arg(&self, i: usize) -> Vec<u8> {
            self.0
                .args()
                .get(i)
                .map(|a| a.as_bytes().to_vec())
                .unwrap_or_default()
        }

        pub(crate) fn listen(&mut self, port: u16) -> i64 {
            self.0.listen(port).map_or(-1, |h| h as i64)
        }

        pub(crate) fn connect(&mut self, host: &str, port: u16) -> i64 {
            self.0.connect(host, port).map_or(-1, |h| h as i64)
        }

        pub(crate) fn take(&mut self, conn: usize, out: &mut [u8]) -> usize {
            self.0.take(conn, out)
        }

        pub(crate) fn write(&mut self, conn: usize, bytes: &[u8]) -> bool {
            self.0.write(conn, bytes).is_ok()
        }

        pub(crate) fn close(&mut self, id: usize) {
            self.0.close(id);
        }

        pub(crate) fn watch(&mut self, id: usize) {
            self.0.watch(id);
        }

        pub(crate) fn watch_signals(&mut self) {
            self.0.watch_signals();
        }

        pub(crate) fn waiting(&self) -> bool {
            self.0.waiting()
        }

        pub(crate) fn poll(&mut self, wait: Option<std::time::Duration>) -> Vec<Event> {
            self.0
                .poll(wait)
                .into_iter()
                .map(|e| match e {
                    moddil_os::Event::Accepted(l, c) => Event::Accepted(l, c),
                    moddil_os::Event::Data(c, n) => Event::Data(c, n),
                    moddil_os::Event::Ended(c) => Event::Ended(c),
                    moddil_os::Event::Signal(s) => Event::Signal(s),
                })
                .collect()
        }
    }

    /// When the run began, for the wall clock.
    pub(crate) struct Start(std::time::Instant);

    impl Start {
        pub(crate) fn now() -> Start {
            Start(std::time::Instant::now())
        }

        /// Nanoseconds since the run began.
        pub(crate) fn elapsed(&self) -> i64 {
            i64::try_from(self.0.elapsed().as_nanos()).unwrap_or(i64::MAX)
        }
    }

    pub(crate) fn random() -> i64 {
        moddil_os::random()
    }

    pub(crate) fn wall_clock() -> i64 {
        moddil_os::wall_clock()
    }

    /// No GPU natively yet (`gpu.1`): nothing is dispatched.
    pub(crate) fn gpu_dispatch(_: &str, _: &[u8], _: i64, _: i64, _: &mut [u32], _: u32) -> bool {
        false
    }

    /// No page natively: nothing is shown (`gpu.1`).
    pub(crate) fn gpu_show(_: &[u32], _: i64, _: i64) {}

    /// No window natively: a frame asks to stop (`gpu.1`).
    pub(crate) fn gpu_frame(out: &mut [i64; 8]) {
        *out = [0, 0, 0, 0, 0, 0, 0, 1];
    }

    pub(crate) fn write_stderr(text: &str) {
        moddil_os::write_stderr(text);
    }
}

#[cfg(target_arch = "wasm32")]
mod host {
    use super::Event;

    #[link(wasm_import_module = "host")]
    #[allow(unsafe_code)]
    unsafe extern "C" {
        fn moddil_host_os_arg_count() -> u32;
        fn moddil_host_os_arg_len(i: u32) -> u32;
        fn moddil_host_os_arg_copy(i: u32, out: *mut u8);
        fn moddil_host_os_write_stderr(p: *const u8, n: u32);
        fn moddil_host_os_random() -> f64;
        fn moddil_host_os_now() -> f64;
        fn moddil_host_os_wall_clock() -> f64;
        fn moddil_host_os_listen(port: u32) -> i32;
        fn moddil_host_os_connect(p: *const u8, n: u32, port: u32) -> i32;
        fn moddil_host_os_take(conn: u32, out: *mut u8, n: u32) -> u32;
        fn moddil_host_os_write(conn: u32, p: *const u8, n: u32) -> u32;
        fn moddil_host_os_close(id: u32);
        fn moddil_host_os_watch(id: u32);
        fn moddil_host_os_watch_signals();
        fn moddil_host_os_waiting() -> u32;
        /// Waits up to `ms` milliseconds (forever if negative) for an event, suspending the
        /// run (JSPI): how many events there are.
        fn moddil_host_os_poll(ms: f64) -> u32;
        /// Event `i` of the last poll: its kind (0 accepted, 1 data, 2 ended, 3 signal) and
        /// its two numbers.
        fn moddil_host_os_event(i: u32, out: *mut f64) -> u32;
        /// Runs a kernel (`gpu.1`): WGSL `source`, `params`, over a `width` by `height` image,
        /// which `mode` says to show (1) and to read back into `out` (2); suspends (JSPI) until
        /// it has. 1 if it could.
        fn moddil_host_gpu_dispatch(
            source: *const u8,
            source_len: u32,
            params: *const u8,
            params_len: u32,
            width: u32,
            height: u32,
            out: *mut u32,
            out_len: u32,
            mode: u32,
        ) -> u32;
        /// Shows `n` pixels at `p`, a `width` by `height` image, in the page (`gpu.1`).
        fn moddil_host_gpu_show(p: *const u32, n: u32, width: u32, height: u32);
        /// Waits for the next animation frame (JSPI), then writes its 8 words to `out`
        /// (`gpu.1`).
        fn moddil_host_gpu_frame(out: *mut i64);
    }

    /// The OS: the host's.
    pub(crate) struct Os;

    #[allow(unsafe_code)]
    impl Os {
        pub(crate) fn new(_: Vec<String>) -> Os {
            Os
        }

        pub(crate) fn arg_count(&self) -> usize {
            // SAFETY: the host's function takes and gives numbers.
            unsafe { moddil_host_os_arg_count() as usize }
        }

        pub(crate) fn arg(&self, i: usize) -> Vec<u8> {
            // SAFETY: the buffer is as long as the host says the argument is.
            unsafe {
                let n = moddil_host_os_arg_len(i as u32) as usize;
                let mut v = vec![0u8; n];
                moddil_host_os_arg_copy(i as u32, v.as_mut_ptr());
                v
            }
        }

        pub(crate) fn listen(&mut self, port: u16) -> i64 {
            // SAFETY: as above.
            i64::from(unsafe { moddil_host_os_listen(u32::from(port)) })
        }

        pub(crate) fn connect(&mut self, host: &str, port: u16) -> i64 {
            // SAFETY: the text lives across the call.
            i64::from(unsafe {
                moddil_host_os_connect(host.as_ptr(), host.len() as u32, u32::from(port))
            })
        }

        pub(crate) fn take(&mut self, conn: usize, out: &mut [u8]) -> usize {
            // SAFETY: the host writes at most `out.len()` bytes.
            unsafe { moddil_host_os_take(conn as u32, out.as_mut_ptr(), out.len() as u32) as usize }
        }

        pub(crate) fn write(&mut self, conn: usize, bytes: &[u8]) -> bool {
            // SAFETY: the bytes live across the call.
            unsafe { moddil_host_os_write(conn as u32, bytes.as_ptr(), bytes.len() as u32) != 0 }
        }

        pub(crate) fn close(&mut self, id: usize) {
            // SAFETY: as above.
            unsafe { moddil_host_os_close(id as u32) }
        }

        pub(crate) fn watch(&mut self, id: usize) {
            // SAFETY: as above.
            unsafe { moddil_host_os_watch(id as u32) }
        }

        pub(crate) fn watch_signals(&mut self) {
            // SAFETY: as above.
            unsafe { moddil_host_os_watch_signals() }
        }

        pub(crate) fn waiting(&self) -> bool {
            // SAFETY: as above.
            unsafe { moddil_host_os_waiting() != 0 }
        }

        pub(crate) fn poll(&mut self, wait: Option<std::time::Duration>) -> Vec<Event> {
            let ms = wait.map_or(-1.0, |w| w.as_secs_f64() * 1000.0);
            // SAFETY: the host fills three numbers for each event.
            unsafe {
                let n = moddil_host_os_poll(ms);
                (0..n)
                    .map(|i| {
                        let mut out = [0f64; 2];
                        let kind = moddil_host_os_event(i, out.as_mut_ptr());
                        let (a, b) = (out[0] as usize, out[1] as usize);
                        match kind {
                            0 => Event::Accepted(a, b),
                            1 => Event::Data(a, b),
                            2 => Event::Ended(a),
                            _ => Event::Signal(out[0] as i64),
                        }
                    })
                    .collect()
            }
        }
    }

    /// When the run began, for the wall clock: the host's milliseconds then.
    pub(crate) struct Start(f64);

    #[allow(unsafe_code)]
    impl Start {
        pub(crate) fn now() -> Start {
            // SAFETY: as above.
            Start(unsafe { moddil_host_os_now() })
        }

        /// Nanoseconds since the run began.
        pub(crate) fn elapsed(&self) -> i64 {
            // SAFETY: as above.
            ((unsafe { moddil_host_os_now() } - self.0) * 1_000_000.0) as i64
        }
    }

    #[allow(unsafe_code)]
    pub(crate) fn random() -> i64 {
        // SAFETY: as above.
        unsafe { moddil_host_os_random() as i64 }
    }

    /// The host's real time, as nanoseconds since the epoch (to its milliseconds).
    #[allow(unsafe_code)]
    pub(crate) fn wall_clock() -> i64 {
        // SAFETY: as above.
        unsafe { moddil_host_os_wall_clock() as i64 }
    }

    #[allow(unsafe_code)]
    pub(crate) fn write_stderr(text: &str) {
        // SAFETY: the text lives across the call.
        unsafe { moddil_host_os_write_stderr(text.as_ptr(), text.len() as u32) }
    }

    #[allow(unsafe_code)]
    pub(crate) fn gpu_dispatch(
        source: &str,
        params: &[u8],
        width: i64,
        height: i64,
        out: &mut [u32],
        mode: u32,
    ) -> bool {
        // SAFETY: each buffer lives across the call, and the host writes at most `out.len()`
        // words.
        unsafe {
            moddil_host_gpu_dispatch(
                source.as_ptr(),
                source.len() as u32,
                params.as_ptr(),
                params.len() as u32,
                width as u32,
                height as u32,
                out.as_mut_ptr(),
                out.len() as u32,
                mode,
            ) != 0
        }
    }

    #[allow(unsafe_code)]
    pub(crate) fn gpu_show(pixels: &[u32], width: i64, height: i64) {
        // SAFETY: the pixels live across the call; the host only reads them.
        unsafe {
            moddil_host_gpu_show(
                pixels.as_ptr(),
                pixels.len() as u32,
                width as u32,
                height as u32,
            )
        }
    }

    #[allow(unsafe_code)]
    pub(crate) fn gpu_frame(out: &mut [i64; 8]) {
        // SAFETY: the host writes 8 words.
        unsafe { moddil_host_gpu_frame(out.as_mut_ptr()) }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) use host::{
    Os, Start, gpu_dispatch, gpu_frame, gpu_show, random, wall_clock, write_stderr,
};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::{
    Os, Start, gpu_dispatch, gpu_frame, gpu_show, random, wall_clock, write_stderr,
};
