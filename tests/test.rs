#[macro_use] extern crate lazy_static;
extern crate panic_monitor;

use panic_monitor::PanicMonitor;
use std::thread::{self, ThreadId};
use std::time::Duration;

lazy_static! {
    static ref PANIC_MONITOR: PanicMonitor = PanicMonitor::new();
}

#[test]
fn test() {
    // Initialise the PanicMonitor
    PANIC_MONITOR.init();

    let good = thread::spawn(|| { thread::sleep(Duration::from_millis(100)); }).thread().id();
    let bad = thread::spawn( || { thread::sleep(Duration::from_millis(100)); panic!(); }).thread().id();
    let watcher = thread::spawn(move || {
        let t = PANIC_MONITOR.wait(&[good, bad]);
        let t: Vec<ThreadId> = t.iter().map(|x|x.id()).collect();
        assert_eq!(t, vec![bad]);
        thread::sleep(Duration::from_millis(100));
        let t = PANIC_MONITOR.wait(&[good, bad]);
        let t: Vec<ThreadId> = t.iter().map(|x|x.id()).collect();
        assert_eq!(t, vec![bad]);
    });

    watcher.join().unwrap();
}
