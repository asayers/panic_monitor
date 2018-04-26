/*! A barrier which blocks until a watched thread panics.

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

use std::collections::HashSet;
use std::panic;
use std::thread::{self, ThreadId};
use std::sync::*;
use std::time::*;

pub struct PanicBarrier {
    panicked: Mutex<HashSet<ThreadId>>,   // Threads which have panicked
    cvar: Condvar,
}

impl PanicBarrier {
    pub fn new() -> PanicBarrier {
        PanicBarrier {
            panicked: Mutex::new(HashSet::new()),
            cvar: Condvar::new(),
        }
    }

    /// Intialise the `PanicBarrier`.  This registers a panic handler which marks the barrier as
    /// complete and signals all threads waiting on the panic barrier.
    pub fn init(&'static self) {
        let hook = panic::take_hook();
        panic::set_hook(Box::new(move|x| {
            let mut panicked = self.panicked.lock().unwrap();
            panicked.insert(thread::current().id());
            self.cvar.notify_all();
            hook(x);
        }));
    }

    /// Block the current thread until some other thread panics.
    pub fn wait(&self, watch_list: &[ThreadId]) -> Vec<ThreadId> {
        let mut panicked = self.panicked.lock().unwrap();

        let watch_list: HashSet<ThreadId> = watch_list.into_iter().map(|x|x.clone()).collect();
        let watched_panicked: Vec<ThreadId> = watch_list.intersection(&panicked).map(|x|x.clone()).collect();
        if watched_panicked.len() > 0 { return watched_panicked; }

        loop {
            panicked = self.cvar.wait(panicked).unwrap();
            let watched_panicked: Vec<ThreadId> = watch_list.intersection(&panicked).map(|x|x.clone()).collect();
            if watched_panicked.len() > 0 { return watched_panicked; }
        }
    }

    /// Block the current thread until some other thread panics.
    pub fn wait_timeout(&self, watch_list: &[ThreadId], dur: Duration) -> Vec<ThreadId> {
        let mut panicked = self.panicked.lock().unwrap();

        let watch_list: HashSet<ThreadId> = watch_list.into_iter().map(|x|x.clone()).collect();
        let watched_panicked: Vec<ThreadId> = watch_list.intersection(&panicked).map(|x|x.clone()).collect();
        if watched_panicked.len() > 0 { return watched_panicked; }

        loop {
            let (guard, res) = self.cvar.wait_timeout(panicked, dur).unwrap();
            panicked = guard;
            let watched_panicked: Vec<ThreadId> = watch_list.intersection(&panicked).map(|x|x.clone()).collect();
            if watched_panicked.len() > 0 || res.timed_out() { return watched_panicked; }
        }
    }
}
