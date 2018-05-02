/*! panic_monitor helps you monitor your threads and deal with panics.

You might be tempted to use libstd's [`JoinHandle`]s for this use-case; however, they have two
major limitations:

 * [`JoinHandle::join`] blocks the current thread.  If you want to monitor multiple threads from a
   single "supervisor" thread, you would need something like `try_join`, and ideally you'd have an
   "epoll for [`JoinHandle`]s" as well to avoid busy-waiting.  [`JoinHandle`] doesn't implement
   these, however.
 * You can't clone a [`JoinHandle`].  If you want multiple threads to be notified when a particular
   thread panics, you can't use its [`JoinHandle`] to achieve it.

panic_monitor handles both of these issues.  [`PanicMonitor::wait`] allows you to specify a number
of threads.  As soon as one of them panics, it returns a [`Thread`] struct (which contains the name
and ID of the panicking thread).  When calling [`PanicMonitor::wait`], you specify the watch-list
in terms of [`ThreadId`]s.  Since these are clonable, mulitple supervisor threads can monitor the
same worker thread.

Some other differences between [`PanicMonitor::wait`] and [`JoinHandle::join`]:

 * You don't receive the value which was passed to [`panic`].  (This would be impossible, given
   that such values are not required to implement [`Clone`].)
 * You aren't notified when a thread shuts down normally.  `PanicMonitor` is for handling
   panicking threads only.

[`PanicMonitor::wait`]: struct.PanicMonitor.html#method.wait
[`JoinHandle`]: https://doc.rust-lang.org/std/thread/struct.JoinHandle.html
[`JoinHandle::join`]: https://doc.rust-lang.org/std/thread/struct.JoinHandle.html#method.join
[`panic`]: https://doc.rust-lang.org/std/macro.panic.html
[`Clone`]: https://doc.rust-lang.org/std/clone/trait.Clone.html
[`Thread`]: https://doc.rust-lang.org/std/thread/struct.Thread.html
[`ThreadId`]: https://doc.rust-lang.org/std/thread/struct.ThreadId.html

## Usage

Create a global [`PanicMonitor`] using [`lazy_static`], and initialise it from your main thread.
Ideally you should do this before spawning any new threads.

[`PanicMonitor`]: struct.PanicMonitor.html
[`lazy_static`]: https://docs.rs/lazy_static/1.0.0/lazy_static/macro.lazy_static.html

```
#[macro_use] extern crate lazy_static;
extern crate panic_monitor;

use panic_monitor::PanicMonitor;
use std::thread;
use std::time::Duration;

lazy_static! {
    static ref PANIC_MONITOR: PanicMonitor = PanicMonitor::new();
}

fn main() {
    // Install a panic hook
    PANIC_MONITOR.init();

    let h = thread::spawn(|| {
        thread::sleep(Duration::from_millis(100));
        panic!();
    });

    PANIC_MONITOR.wait(&[h.thread().id()]);
    // ^ this will block until the thread panics

    PANIC_MONITOR.wait(&[h.thread().id()]);
    // ^ this will return immediately, since the thread is already dead

    h.join().unwrap_err();
}
```
*/

use std::collections::HashMap;
use std::panic;
use std::sync::*;
use std::thread::{self, Thread, ThreadId};
use std::time::*;

const POISON_MSG: &str = "panic_monitor: Inner lock poisoned (please submit a bug report)";

/// A list of all threads which have panicked, with the ability to notify interested parties when
/// this list is updated.
pub struct PanicMonitor {
    panicked: Mutex<HashMap<ThreadId, Thread>>,   // All threads which have historically panicked
    cvar: Condvar,
}

impl PanicMonitor {
    /// Create a new `PanicMonitor`.
    ///
    /// Call this inside a [`lazy_static`] block.  You must call [`init`] after this.
    ///
    /// [`init`]: #method.init
    /// [`lazy_static`]: https://docs.rs/lazy_static/1.0.0/lazy_static/macro.lazy_static.html
    pub fn new() -> PanicMonitor {
        PanicMonitor {
            panicked: Mutex::new(HashMap::new()),
            cvar: Condvar::new(),
        }
    }

    /// Initialise the `PanicMonitor`.
    ///
    /// Call this method as early as you can: a thread which panics before the `PanicMonitor` is
    /// initialised will not trigger wake-ups.  Calling `init` multiple times is relatively
    /// harmless.
    //
    // If you need to uninstall some existing handlers by calling `std::panic::set_hook(|_| {})`,
    // or something, you can call `init` again afterwards to re-add `PanicMonitor`'s hook.
    pub fn init(&'static self) {
        // Install a panic hook which makes a record of the panicking thread and notifies all
        // threads waiting on the PanicMonitor
        let hook = panic::take_hook();
        panic::set_hook(Box::new(move|x| {
            let mut panicked = self.panicked.lock().expect(POISON_MSG);
            let current = thread::current();
            panicked.insert(current.id(), current);
            self.cvar.notify_all();
            hook(x);
        }));
    }

    /// Block the current thread until one of the watched threads panics.  The returned vector is
    /// always non-empty.
    ///
    /// Note that this function returns as soon as one or more of the threads on the watch list has
    /// panicked.  This means that if you specify a thread which has already panicked, this
    /// function will return immediately.  Think of it as level-triggered, not edge-triggered.
    pub fn wait(&self, watch_list: &[ThreadId]) -> Vec<Thread> {
        let mut watched_panicked = vec![];
        let mut panicked = self.panicked.lock().expect(POISON_MSG);
        loop {
            for tid in watch_list {
                if let Some(t) = panicked.get(tid) {
                    watched_panicked.push(t.clone());
                }
            }
            if watched_panicked.len() > 0 { return watched_panicked; }
            panicked = self.cvar.wait(panicked).expect(POISON_MSG);
        }
    }

    /// Block the current thread until one of the watched threads panic, or the timeout expires.
    /// The returned vector is empty if and only if the timeout expired.
    ///
    /// See [`wait`] for more information.
    ///
    /// [`wait`]: #method.wait
    pub fn wait_timeout(&self, watch_list: &[ThreadId], mut dur: Duration) -> Vec<Thread> {
        let mut watched_panicked = vec![];
        let mut panicked = self.panicked.lock().expect(POISON_MSG);
        loop {
            for tid in watch_list {
                if let Some(t) = panicked.get(tid) {
                    watched_panicked.push(t.clone());
                }
            }
            if watched_panicked.len() > 0 { return watched_panicked; }
            let now = Instant::now();
            let (guard, res) = self.cvar.wait_timeout(panicked, dur).expect(POISON_MSG);
            let elapsed = now.elapsed();
            panicked = guard;
            if res.timed_out() || elapsed >= dur { return vec![]; }
            dur -= elapsed; // safe because ^
        }
    }

    /// Check if any of the specified threads have panicked.  This function may block, but only
    /// very briefly.  The returned vector may be empty.
    ///
    /// See [`wait`] for more information.
    ///
    /// [`wait`]: #method.wait
    pub fn check(&self, watch_list: &[ThreadId]) -> Vec<Thread> {
        let mut watched_panicked = vec![];
        let panicked = self.panicked.lock().expect(POISON_MSG);
        for tid in watch_list {
            if let Some(t) = panicked.get(tid) {
                watched_panicked.push(t.clone());
            }
        }
        watched_panicked
    }
}
