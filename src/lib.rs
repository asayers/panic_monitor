/*! A barrier which blocks until any thread panics.

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

    // Spawn some threads
    let x = thread::spawn(|| { thread::sleep(Duration::from_millis(100)); panic!(); });

    // Wait until a thread panics
    PANIC_BARRIER.wait();
}
```
*/

use std::panic;
use std::sync::*;
use std::time::*;

pub struct PanicBarrier {
    lock: Mutex<bool>,
    cvar: Condvar,
}

impl PanicBarrier {
    pub fn new() -> PanicBarrier {
        PanicBarrier {
            lock: Mutex::new(false),
            cvar: Condvar::new(),
        }
    }

    /// Intialise the `PanicBarrier`.  This registers a panic handler which marks the barrier as
    /// complete and signals all threads waiting on the panic barrier.
    pub fn init(&'static self) {
        let hook = panic::take_hook();
        panic::set_hook(Box::new(move|x| {
            let mut panicking = self.lock.lock().unwrap();
            *panicking = true;
            self.cvar.notify_all();
            hook(x);
        }));
    }

    /// Block the current thread until some other thread panics.
    pub fn wait(&self) {
        let mut panicking = self.lock.lock().unwrap();
        while !*panicking { panicking = self.cvar.wait(panicking).unwrap(); }
    }

    /// Block the current thread until some other thread panics.
    pub fn wait_timeout(&self, dur: Duration) -> WaitTimeoutResult {
        let mut panicking = self.lock.lock().unwrap();
        loop {
            let (guard, res) = self.cvar.wait_timeout(panicking, dur).unwrap();
            panicking = guard;
            if *panicking || res.timed_out() { return res; }
        }
    }
}
