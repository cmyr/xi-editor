use std::time::Duration;
use std::collections::{BTreeSet, BTreeMap};
//use std::sync::atomic::{AtomicUsize, Ordering};
use std::borrow::Cow;
//use std::mem;
use std::fmt;
use std::cmp::Ordering;
use std::ops::{Add, Sub};

use libc;

pub type CowStr = Cow<'static, str>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SysTime(u64);

impl SysTime {

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    pub fn now() -> Self {
        let t = unsafe { libc::mach_absolute_time() };
        SysTime(t as u64)
    }

    #[cfg(target_os = "unix")]
    pub fn now() -> Self {

        let mut t = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };

       let success = unsafe { libc::clock_gettime(clock, &mut t) };
       if success < 0 {
           eprintln!("xi-core failed to get system time.");
       }
       SysTime(t.tv_sec * 1_000_000_000 + t.tv_nsec)
    }
}

impl PartialEq for SysTime {
    fn eq(&self, other: &SysTime) -> bool {
        self.0 == other.0
    }
}

impl Eq for SysTime {}

impl PartialOrd for SysTime {
    fn partial_cmp(&self, other: &SysTime) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

impl Ord for SysTime {
    fn cmp(&self, other: &SysTime) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl Add for SysTime {
    type Output = SysTime;

    fn add(self, rhs: SysTime) -> SysTime {
        SysTime(self.0 + rhs.0)
    }
}

impl Sub for SysTime {
    type Output = SysTime;

    fn sub(self, rhs: SysTime) -> SysTime {
        SysTime(self.0 - rhs.0)
    }
}

pub fn merge_traces(traces: &mut [BTreeMap<usize, Trace>]) {
    let all_ids = traces.iter()
        .flat_map(|t| t.keys().cloned())
        .collect::<BTreeSet<_>>();

    for id in all_ids.iter() {
        let trace_groups = traces.iter_mut()
            .flat_map(|t| t.remove(id))
            .collect::<Vec<_>>();

        let min_t = trace_groups.iter()
            .map(|g| g.start)
            .min()
            .unwrap();

        let mut out = Vec::new();
        for group in trace_groups {
            let glabel = group.label.clone().unwrap_or("".to_owned());
            for event in group.events {
                let d = PrettyDuration::from_nanos(event.time.0 - min_t.0);
                let s = format!("{:>5} {}.{}.{}", d, group.src_name, glabel, event.label);
                out.push((event.time, s));
            }
        }

        out.sort();

        let ready_to_print: Vec<String> = out.drain(..)
            .map(|(_, s)| s).collect();

        eprintln!("##### trace {} #####\n{}", id, ready_to_print.join("\n"));

    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub trace_id: usize,
    pub src_name: String,
    pub label: Option<String>,
    // ns since unix epoch
    pub start: SysTime,
    pub events: Vec<Event>,
}

impl Trace {
    pub fn new(trace_id: usize, src_name: String) -> Self {
        let start = SysTime::now();
        let events = vec![Event { label: "received".into(), time: start }];
        let label = None;
        Trace { trace_id, src_name, label, start, events }
    }

    pub fn add_event<S: Into<CowStr>>(&mut self, label: S) {
        let label = label.into();
        let time = SysTime::now();
        self.events.push( Event { label, time } );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    label: CowStr,
    time: SysTime,
}

fn nanos_from_duration(d: &Duration) -> u64 {
    d.as_secs() * 1_000_000_000 + d.subsec_nanos() as u64
}

struct PrettyDuration {
    secs: u64,
    millis: u64,
    micros: u64,
    nanos: u64,
}

impl PrettyDuration {
    pub fn new(d: &Duration) -> Self {
        let d = nanos_from_duration(d);
        Self::from_nanos(d)
    }

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
