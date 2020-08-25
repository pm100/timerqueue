use log::trace;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError::{Disconnected, Timeout};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub struct TimerQueue {
    jh: Option<JoinHandle<()>>,
    tx: Sender<QueueInstruction>,
}

pub type TQIFunc = Box<dyn Fn() -> () + Send + Sync>;
struct TimerQueueItem {
    when: Instant, // when it should run
    name: String,  // for trace only
    what: TQIFunc, // what to run
    handle: TimerQueueHandle,
}

#[derive(Clone, Debug)]
pub struct TimerQueueHandle {
    handle: Arc<(Mutex<TQHState>, Condvar)>,
}
#[derive(Debug,Clone,PartialEq)]
enum TQHState{
    NotValid,
    WaitingToRun,
    Running,
    Finished,
    Final
}
impl TimerQueueHandle {
    pub fn new() -> Self {
        Self {
            handle: Arc::new((Mutex::new(TQHState::NotValid), Condvar::new())),
        }
    }

    pub fn is_valid(&self) -> bool{
        let (lock, _cv) = &*self.handle;
        let state = lock.lock().unwrap();
        *state != TQHState::NotValid 
    }
    pub fn wait(& self) {
        let (lock, cv) = &*self.handle;
        let mut state = lock.lock().unwrap();
        assert!(*state != TQHState::NotValid && *state != TQHState::Final);
        while *state != TQHState::Finished {
            state = cv.wait(state).unwrap();
        }
        *state = TQHState::Final;
    }
    pub fn finished(& self) -> bool {
        let (lock, _cv) = &*self.handle;
        let  state = lock.lock().unwrap();
        assert!(*state != TQHState::NotValid);
        let finished = *state == TQHState::Finished || *state == TQHState::Final;
      
        finished
    }
    fn signal(&self, new_state:TQHState) {
        let (lock, cv) = &*self.handle;
        let mut state = lock.lock().unwrap();
        *state = new_state;
        cv.notify_all();
    }
}

enum QueueInstruction {
    Do(TimerQueueItem),
    Stop,
}

// all these Trait impls are required so that binaryheap can sort on due time
// ====================================================
impl Ord for TimerQueueItem {
    fn cmp(&self, other: &TimerQueueItem) -> Ordering {
        other.when.cmp(&self.when)
    }
}
impl PartialOrd for TimerQueueItem {
    fn partial_cmp(&self, other: &TimerQueueItem) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for TimerQueueItem {
    fn eq(&self, other: &Self) -> bool {
        self.when == other.when
    }
}
impl Eq for TimerQueueItem {}
// ====================================================

enum GetItemResult {
    Timeout,
    Stop,
    NewItem(TimerQueueItem),
}
// either we got a new tqi, were told to stop or timed out
// timeout indicates that we need to (probably ) run the top of the queue
// called in one of 2 ways - duration == 0 , wait forever, duration != 0, wait that long
// maybe should be an option
fn get_item(rx: &Receiver<QueueInstruction>, wait: Duration) -> GetItemResult {
    let mut stop = false;
    let mut timeout = false;
    let mut qinst = QueueInstruction::Stop; // init to something
    if wait == Duration::from_secs(0) {
        match rx.recv() {
            Ok(qinstx) => qinst = qinstx,
            Err(_) => stop = true,
        };
    } else {
        match rx.recv_timeout(wait) {
            Ok(qinstx) => qinst = qinstx,
            Err(e) => match e {
                Timeout => timeout = true,
                Disconnected => stop = true,
            },
        };
    }
    if stop {
        GetItemResult::Stop
    } else if timeout {
        GetItemResult::Timeout
    } else {
        match qinst {
            QueueInstruction::Do(tqi) => {
                trace!(target:"TimerQueue","got item {}", tqi.name);
                GetItemResult::NewItem(tqi)
            }
            QueueInstruction::Stop => {
                trace!("got stop request");
                GetItemResult::Stop
            }
        }
    }
}

impl TimerQueue {
    pub fn new() -> TimerQueue {
        let (tx, rx): (Sender<QueueInstruction>, Receiver<QueueInstruction>) = mpsc::channel();
        let jh = thread::spawn(move || {
            let mut queue: BinaryHeap<TimerQueueItem> = BinaryHeap::new();
            let mut stop = false;
            loop {
                if queue.len() == 0 {
                    if stop {
                        break;
                    }
                    match get_item(&rx, Duration::from_secs(0)) {
                        GetItemResult::Stop => {
                            break; // no work and a request to stop - so stop
                        }
                        GetItemResult::NewItem(i) => {
                            queue.push(i);
                        }
                        GetItemResult::Timeout => panic!(), // cannot actually happen here
                    }
                } else {
                    let now = Instant::now();
                    let tqi = queue.pop().expect("oops");
                    let due = tqi.when;
                    if due > now {
                        let wait = due - now;
                        queue.push(tqi);

                        trace!(target:"TimerQueue","sleep for {0}ms", wait.as_millis());
                        match get_item(&rx, wait) {
                            GetItemResult::Stop => {
                                stop = true; // drain the queue first
                            }
                            GetItemResult::NewItem(i) => {
                                queue.push(i);
                            }
                            GetItemResult::Timeout => {
                                continue;
                            }
                        }
                    } else {
                        trace!(target:"TimerQueue","running {0}", tqi.name);
                        tqi.handle.signal(TQHState::Running);
                        (tqi.what)();
                        tqi.handle.signal(TQHState::Finished);
                        // queue.pop().unwrap();
                    }
                }
            }
            trace!(target:"TimerQueue", "thread completed");
        });

        TimerQueue { jh: Some(jh), tx }
    }

    pub fn queue(&self, f: TQIFunc,  n: String, when: Instant) -> TimerQueueHandle {
        let handle = TimerQueueHandle::new();
        trace!(target:"TimerQueue", "queued {0}", &n);
        let qi = TimerQueueItem {
            what: f,
            name: n,
            when: when,
            handle: handle.clone(),
        };
        let qinst = QueueInstruction::Do(qi);
        self.tx.send(qinst).unwrap();
        handle.signal(TQHState::WaitingToRun);
        handle
    }
}

impl Drop for TimerQueue {
    fn drop(&mut self) {
        self.tx.send(QueueInstruction::Stop).unwrap();
        match self.jh.take() {
            Some(jh) => jh.join().unwrap(),
            None => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use crate::*;

    struct TestObj{}

    impl TestObj {
        pub fn f1(&self, u: u32) {
            println!("{}", u);
        }
        pub fn f2(&self, s: String) {
            println!("{}", s);
        }
    }

    #[test]
    fn it_works() {
        env_logger::init();
        let o1 = Arc::new(TestObj {});
        let o2 = Arc::new(TestObj {});
        let tq = TimerQueue::new();
        let oc1 = o1.clone();
        tq.queue(
            Box::new(move || oc1.f1(1)),
            String::from("1 sec"),
            Instant::now() + Duration::from_millis(1000),
        );
        let oc2 = o2.clone();

        let h5 = tq.queue(
            Box::new(move || oc2.f2("5".to_string())),
            String::from("5 sec"),
            Instant::now() + Duration::from_millis(5000),
        );
        let oc2 = o2.clone();
        tq.queue(
            Box::new(move || oc2.f1(2)),
            String::from("2 sec"),
            Instant::now() + Duration::from_millis(2000),
        );
        let oc1 = o1.clone();
        let h3 = tq.queue(
            Box::new(move || oc1.f2("3".to_string())),
            String::from("3 sec"),
            Instant::now() + Duration::from_millis(3000),
        );

        h3.wait();
        println!("h3 done");
        println!("h5 is finished {}", h5.finished());
        h5.wait();
        println!("h5 is finished {}", h5.finished());
    }
}
