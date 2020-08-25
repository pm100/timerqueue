# timerqueue
My first rust library

Purpose. Allow a system to create a queue of tasks to be run asynchronously at some specified point in the future.

The client is returned a handle when the task is queued, this can be queried to see if the task has finished

sample use from the test 

    struct TestObj{}

    impl TestObj {
        pub fn f1(&self, u: u32) {
            println!("{}", u);
        }
        pub fn f2(&self, s: String) {
            println!("{}", s);
        }
    }

        let o1 = Arc::new(TestObj {});
        let tq = TimerQueue::new();
        let oc1 = o1.clone();
        tq.queue(
            Box::new(move || oc1.f1(1)),
            String::from("1 sec"),
            Instant::now() + Duration::from_millis(1000),
        );

obviously the TimerQueue object would have been created at a larger scope in a real use case.

