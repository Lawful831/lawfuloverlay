//! Failure reports for the server's log.
//!
//! Anything that fails on this machine (voice, screen sampling, graphics,
//! updates, panics) is queued here and the socket thread sends it to the server
//! as `{"type":"telemetry",...}`. Reports wait in the queue while offline, and a
//! crash is written to a file and sent on the next start. Only a short event
//! name and error text go out: never chat content, never the token.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Reports kept while the socket is down; the oldest are dropped first.
const QUEUE_MAX: usize = 50;
/// The same report is not repeated inside this window.
const REPEAT_WINDOW: Duration = Duration::from_secs(60);
const FIELD_CHARS: usize = 300;

struct Queue {
    pending: VecDeque<(String, String)>,
    recent: Vec<(String, String, Instant)>,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue { pending: VecDeque::new(), recent: Vec::new() });

fn clip(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(FIELD_CHARS).collect()
}

/// Queue a failure report. Cheap and never blocks on the network.
pub fn report(event: &str, detail: &str) {
    let (event, detail) = (clip(event), clip(detail));
    let Ok(mut q) = QUEUE.lock() else { return };
    let now = Instant::now();
    q.recent.retain(|(.., at)| now.duration_since(*at) < REPEAT_WINDOW);
    if q.recent.iter().any(|(e, d, _)| *e == event && *d == detail) {
        return;
    }
    q.recent.push((event.clone(), detail.clone(), now));
    if q.pending.len() >= QUEUE_MAX {
        q.pending.pop_front();
    }
    q.pending.push_back((event, detail));
}

/// Reports waiting to go out, oldest first. The caller puts them back with
/// [`requeue`] if the send fails.
pub fn take() -> Vec<(String, String)> {
    QUEUE.lock().map(|mut q| q.pending.drain(..).collect()).unwrap_or_default()
}

/// Put reports back at the front after a failed send.
pub fn requeue(items: Vec<(String, String)>) {
    let Ok(mut q) = QUEUE.lock() else { return };
    for item in items.into_iter().rev() {
        if q.pending.len() < QUEUE_MAX {
            q.pending.push_front(item);
        }
    }
}

/// The frame sent for one report.
pub fn frame(event: &str, detail: &str) -> String {
    serde_json::json!({
        "type": "telemetry",
        "event": event,
        "detail": detail,
        "version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
    })
    .to_string()
}

fn crash_file() -> PathBuf {
    crate::config::config_path().with_file_name("pending_crash.txt")
}

/// Route panics to the queue and to disk, so a crash reaches the server on the
/// next start. Call once, early in `main`.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let text = format!("thread {}: {info}", thread.name().unwrap_or("?"));
        report("panic", &text);
        let _ = std::fs::write(crash_file(), clip(&text));
        previous(info);
    }));
}

/// Queue the crash left by the previous run, if any.
pub fn report_previous_crash() {
    let path = crash_file();
    if let Ok(text) = std::fs::read_to_string(&path) {
        report("previous_crash", &text);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_are_dropped_and_old_reports_age_out() {
        let _ = take();
        report("x", "same");
        report("x", "same");
        report("x", "other");
        assert_eq!(take().len(), 2);
        for i in 0..QUEUE_MAX + 5 {
            report("y", &i.to_string());
        }
        assert_eq!(take().len(), QUEUE_MAX);
    }

    #[test]
    fn frame_has_the_fields_the_server_reads() {
        let v: serde_json::Value = serde_json::from_str(&frame("e", "d")).unwrap();
        assert_eq!(v["type"], "telemetry");
        assert_eq!(v["event"], "e");
        assert!(v["version"].is_string() && v["os"].is_string());
    }
}
