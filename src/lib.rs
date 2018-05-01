/*! A barrier which blocks until a watched thread panics.

Create a global panic barrier using `lazy_static`, and initialise it from your main thread.
Threads can thenceforce use it to wait for other threads to panic.

`PanicBarrier::wait()` allows you to specify a number of threads, and it returns as soon as one of
them panics.  Threads are specified by their `ThreadId` (which is clonable), meaning that mulitple
threads can monitor the same thread.  Each call to `PanicBarrier::wait()` can specify a different
set of watched threads.

When a watched thread panics, you get a `Thread` struct back (which contains the thread's name and
ID).  In contrast with `JoinHandle::join()`, you *don't* get the value which was passed to
`panic!()` - this is not possible, given that this value is not required to implement `Clone`.

```
#[macro_use] extern crate lazy_static;
extern crate panic_barrier;

use panic_barrier::PanicBarrier;
use std::thread;
use std::time::Duration;

lazy_static! {
    static ref PANIC_BARRIER: PanicBarrier = PanicBarrier::new();
}

fn main() {
    // Initialise the PanicBarrier
    PANIC_BARRIER.init();

    let h1 = thread::spawn(|| {
        thread::sleep(Duration::from_millis(100));
        panic!();
    });

    let h2 = thread::spawn(move || {
        PANIC_BARRIER.wait(&[h1.thread().id()]);
        // ^ this will block until thread 1 panicks
        PANIC_BARRIER.wait(&[h1.thread().id()]);
        // ^ this will return immediately, since thread 1 is already dead
    });

    h2.join().unwrap();
}
```
*/

use std::collections::HashMap;
use std::panic;
use std::sync::*;
use std::thread::{self, Thread, ThreadId};
use std::time::*;

const POISON_MSG: &str = "panic_barrier: Inner lock poisoned (please submit a bug report)";

pub struct PanicBarrier {
    panicked: Mutex<HashMap<ThreadId, Thread>>,   // All threads which have historically panicked
    cvar: Condvar,
}

impl PanicBarrier {
    pub fn new() -> PanicBarrier {
        PanicBarrier {
            panicked: Mutex::new(HashMap::new()),
            cvar: Condvar::new(),
        }
    }

    /// Intialise the `PanicBarrier`.  This registers a panic handler which marks the current
    /// thread as panicked and signals all threads waiting on the panic barrier.
    ///
    /// Initialise the `PanicBarrier` before spawning threads.  A thread which panics before the
    /// `PanicBarrier` is initialised will not trigger wake-ups.
    ///
    /// Calling `PanicBarrier::init()` multiple times is reasonably harmless.  If for some reason
    /// you want to throw away existing handlers by calling `std::panic::set_hook()`, you can then
    /// call `PanicBarrier::init()` again to re-add PanicBarrier's hook.
    pub fn init(&'static self) {
        let hook = panic::take_hook();
        panic::set_hook(Box::new(move|x| {
            let mut panicked = self.panicked.lock().expect(POISON_MSG);
            let current = thread::current();
            panicked.insert(current.id(), current);
            self.cvar.notify_all();
            hook(x);
        }));
    }

    /// Block the current thread until one of the watched threads panic.  The returned vector will
    /// always be non-empty.
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
    /// The returned vector will be empty if and only if the timeout expired.
    ///
    /// Note that this function returns as soon as one or more of the threads on the watch list has
    /// panicked.  This means that if you specify a thread which has already panicked, this
    /// function will return immediately.  Think of it as level-triggered, not edge-triggered.
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

    /// Check if any of the specified threads have panicked.  This function will not normally
    /// block.
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
