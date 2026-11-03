//! Opt-in frame timing log for diagnosing playback stutter on a machine we do
//! not control.
//!
//! `AEGISUB_GPUI_TRACE=<path>` writes one line per playback event with the
//! microseconds since the first traced call. When the variable is unset every
//! call is a cheap branch and nothing is written, so release builds pay
//! nothing for it.

use std::fmt::Arguments;
use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();
static ORIGIN: OnceLock<Instant> = OnceLock::new();

fn log() -> &'static Option<Mutex<File>> {
    LOG.get_or_init(|| {
        let path = std::env::var_os("AEGISUB_GPUI_TRACE")?;
        let file = File::create(path).ok()?;
        let _ = ORIGIN.set(Instant::now());
        Some(Mutex::new(file))
    })
}

pub fn enabled() -> bool {
    log().is_some()
}

/// Microseconds since the first traced call; `0` when tracing is off.
pub fn now() -> u64 {
    log();
    ORIGIN
        .get()
        .map_or(0, |origin| origin.elapsed().as_micros() as u64)
}

pub fn write(args: Arguments<'_>) {
    let Some(lock) = log() else { return };
    if let Ok(mut file) = lock.try_lock() {
        let _ = writeln!(file, "{args}");
        let _ = file.flush();
    }
}

macro_rules! trace {
    ($($arg:tt)*) => {
        if crate::trace::enabled() {
            crate::trace::write(format_args!($($arg)*));
        }
    };
}
pub(crate) use trace;
