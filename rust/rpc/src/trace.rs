use std::borrow::Cow;
use std::fmt;

use libc;

pub type CowStr = Cow<'static, str>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SysTime(u64);

/// System clock time, in nanoseconds.
///
/// Note: We would prefer to use `time::Instant`, but it can't be serialized.
/// Generation of this type is platform dependent, and may not work on all
/// platforms. Implementations are taken from `time::Instant` in the stdlib.
///
/// It is important that timestamps are generated equivelantly (through the
/// same system calls) in all processes participating in tracing.
pub type Timestamp = u64;

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn timestamp_now() -> Timestamp {
    unsafe { libc::mach_absolute_time() }
}

#[cfg(target_os = "unix")]
pub fn timestamp_now() -> Timestamp {

    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    let success = unsafe { libc::clock_gettime(clock, &mut t) };
    if success < 0 { panic!("timestamp_now() failed.") }
    t.tv_sec * 1_000_000_000 + t.tv_nsec
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "unix")))]
pub fn timestamp_now() -> Timestamp {
    panic!("Tracing is not supported on your platform");
}


pub fn merge_traces(mut traces: Vec<Vec<Trace>>) {
    let mut all = traces.iter_mut()
        .fold(Vec::new(), |mut all, mut t| { all.append(&mut t); all } );
    all.sort_by_key(|t| t.timestamp);

    let mut base_t = all.first().as_ref().map(|t| t.timestamp).unwrap_or(0);

    for trace in all {
        if trace.is_orphan() {
            base_t = trace.timestamp;
            eprintln!("\n### new tree ###");
        }
        let d = PrettyDuration::from_nanos(trace.timestamp - base_t);
        eprintln!("{:>5} {}.{}", d, trace.proc_name, trace.label);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub timestamp: Timestamp,
    pub proc_name: CowStr,
    pub label: CowStr,
    pub parent: Option<Timestamp>,
}

impl Trace {
    pub fn new(label: CowStr, parent: Option<Timestamp>,
               timestamp: Option<Timestamp>) -> Self
    {
        let timestamp = timestamp.unwrap_or(timestamp_now());
        // we can update the proc_name when we process traces
        let proc_name = "xi-rpc".into();
        Trace { timestamp, proc_name, label, parent }
    }

    fn is_orphan(&self) -> bool {
        self.parent.is_none() || self.parent == Some(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    label: CowStr,
    time: SysTime,
}

struct PrettyDuration {
    secs: u64,
    millis: u64,
    micros: u64,
    nanos: u64,
}

impl PrettyDuration {
    pub fn from_nanos(d: u64) -> Self {
        let secs = d / 1_000_000_000;
        let d = d - secs * 1_000_000_000;
        let millis = d / 1_000_000;
        let d = d - millis * 1_000_000;
        let micros = d / 1_000;
        let nanos = d - micros * 1_000;
        PrettyDuration { secs, millis, micros, nanos }
    }
}

impl fmt::Display for PrettyDuration {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let text = if self.secs > 0 {
            format!("{}.{}s", self.secs, self.millis / 100)
        } else if self.millis > 0 {
            format!("{}.{}ms", self.millis, self.micros / 100)
        } else if self.micros > 0 {
            format!("{}µs", self.micros)
        } else {
            format!("{}ns", self.nanos)
        };
        f.pad(&text)
    }
}
