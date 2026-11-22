//! Tick watchdog owned by the dedicated-server runtime.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// What the tick loop tells `ServerWatchdog`: when the tick in progress was
/// due (`getNextTickTime`), how long a tick is, and whether the server runs.
pub(super) struct Watchdog {
    /// The thread running the ticks, whose stack the report carries.
    server: horizon_threads::stack_dump::Watched,
    base: Instant,
    next_nanos: AtomicU64,
    tick_nanos: AtomicU64,
    running: AtomicBool,
}
impl Watchdog {
    /// `DedicatedServer.initServer`: a thread that stops a server whose tick
    /// runs `max_tick_time` ms past its due time; none when that is 0 or less.
    /// Started from the thread that runs the ticks.
    pub(super) fn start(max_tick_time: i64) -> Option<Arc<Self>> {
        let max = Duration::from_millis(u64::try_from(max_tick_time).ok().filter(|m| *m > 0)?);
        let watchdog = Arc::new(Self {
            server: horizon_threads::stack_dump::Watched::current(),
            base: Instant::now(),
            next_nanos: AtomicU64::new(0),
            tick_nanos: AtomicU64::new(50_000_000),
            running: AtomicBool::new(true),
        });
        let shared = watchdog.clone();
        std::thread::Builder::new()
            .name("Server Watchdog".into())
            .spawn(move || shared.run(max))
            .ok()?;
        Some(watchdog)
    }

    /// The tick loop is about to run the tick due at `next`.
    pub(super) fn tick_due(&self, next: Instant, tick: Duration) {
        let nanos = next.saturating_duration_since(self.base).as_nanos();
        self.next_nanos
            .store(u64::try_from(nanos).unwrap_or(u64::MAX), Ordering::Relaxed);
        let tick = u64::try_from(tick.as_nanos()).unwrap_or(u64::MAX);
        self.tick_nanos.store(tick, Ordering::Relaxed);
    }

    /// Stop the watchdog thread when the server tick loop exits.
    pub(super) fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// `ServerWatchdog.run`.
    fn run(&self, max: Duration) {
        let max = u64::try_from(max.as_nanos()).unwrap_or(u64::MAX);
        while self.running.load(Ordering::Relaxed) {
            let next = self.next_nanos.load(Ordering::Relaxed);
            let now = u64::try_from(self.base.elapsed().as_nanos()).unwrap_or(u64::MAX);
            let behind = now.saturating_sub(next);
            if behind > max {
                // ServerWatchdog: the server thread's stack first, while it is still stuck.
                let stack = self.server.stack(Duration::from_secs(5));
                let took = format!("{:.2}", behind as f32 / 1e9);
                let should = format!(
                    "{:.2}",
                    self.tick_nanos.load(Ordering::Relaxed) as f32 / 1e9
                );
                super::log_from(
                    "Server Watchdog",
                    "ERROR",
                    &format!("A single server tick took {took} seconds (should be max {should})"),
                );
                super::log_from(
                    "Server Watchdog",
                    "ERROR",
                    "Considering it to be crashed, server will forcibly shutdown.",
                );
                let message = format!(
                    "ServerWatchdog detected that a single server tick took {took} seconds (should be max {should})"
                );
                match horizon_client::crash::report_watchdog(
                    std::path::Path::new("."),
                    "server",
                    message,
                    stack,
                ) {
                    Some(path) => super::log_from(
                        "Server Watchdog",
                        "ERROR",
                        &format!(
                            "This crash report has been saved to: {}",
                            path.canonicalize().unwrap_or(path).display()
                        ),
                    ),
                    None => super::log_from(
                        "Server Watchdog",
                        "ERROR",
                        "We were unable to save this crash report to disk.",
                    ),
                }
                std::process::exit(1);
            }
            // Thread.sleep(nextTickTime + maxTickTime - now).
            let wait = (next + max).saturating_sub(now).max(1_000_000);
            std::thread::sleep(Duration::from_nanos(wait));
        }
    }
}
