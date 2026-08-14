use field_visit_metrics::{report_work_order, DispatchStatus, WorkOrderSnapshot};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match Pin::new(&mut future).poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn main() {
    let snapshot = WorkOrderSnapshot {
        snapshot_id: "2026-08-14T09:00Z".into(),
        work_order_id: "WO-1842".into(),
        territory: "east".into(),
        photo_count: 3,
        dispatch_status: DispatchStatus::Completed,
        technician_follow_up: false,
    };

    match block_on(report_work_order(&snapshot)) {
        Ok(count) => println!("reported {count} metrics for {}", snapshot.work_order_id),
        Err(error) => {
            eprintln!("report failed: {error}");
            std::process::exit(1);
        }
    }
}

