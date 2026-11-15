use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use bonk_hal::FilterOptions;
use serde_json::{Value, json};

use crate::errors::Code;
use crate::list;

use crate::serve::params::WatchParams;
use crate::serve::protocol::{Failure, Out, parse};
use crate::serve::{Config, State};

pub(in crate::serve) struct WatchHandle {
    pub(super) stop: Arc<AtomicBool>,
    pub(super) thread: JoinHandle<()>,
}

pub(in crate::serve) fn start_watch(
    params: &Value,
    cfg: &Config,
    out: &Out,
    state: &mut State,
) -> Result<Value, Failure> {
    let p: WatchParams = parse(params)?;
    stop_watch(state);
    let interval = Duration::from_millis(p.interval_ms.unwrap_or(2000).max(100));
    let options = cfg.view.widen(FilterOptions {
        show_internal: p.show_internal,
        allow_virtual: p.allow_virtual,
    });
    let all = p.all || cfg.view.show_all;
    let (enumerate_fn, out) = (Arc::clone(&cfg.enumerate), out.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        let mut last: Option<Value> = None;
        while !flag.load(Ordering::SeqCst) {
            let current = match enumerate_fn() {
                Ok(disks) => list::report_value(options, all, &disks).unwrap_or_default(),
                Err(e) => {
                    json!({"error": {"code": Code::of_enumeration(&e).id(), "message": e.to_string()}})
                }
            };
            if last.as_ref() != Some(&current) {
                out.send(&json!({"event": "disks_changed", "initial": last.is_none(), "report": current}));
                last = Some(current);
            }
            // Small steps: a stop is seen without waiting the whole interval.
            let mut waited = Duration::ZERO;
            while waited < interval && !flag.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(20));
                waited += Duration::from_millis(20);
            }
        }
    });
    state.watch = Some(WatchHandle { stop, thread });
    Ok(
        json!({"watching": true, "interval_ms": u64::try_from(interval.as_millis()).unwrap_or(u64::MAX)}),
    )
}

pub(in crate::serve) fn stop_watch(state: &mut State) -> bool {
    match state.watch.take() {
        Some(handle) => {
            handle.stop.store(true, Ordering::SeqCst);
            if handle.thread.join().is_err() {
                log::error!(target: "bonk::cli", "the disk watch panicked");
            }
            true
        }
        None => false,
    }
}
