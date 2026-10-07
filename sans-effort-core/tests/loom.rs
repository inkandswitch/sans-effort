//! The driver's waker and outbox under every interleaving loom finds.
//!
//! The host-protocol spec proves that a host which follows `woke` frames never
//! loses a machine; it takes for granted that the driver reports every wake.
//! That is these models' subject: a waker called from another thread while
//! the driver is polling, or between polls, must reach the hook — once per
//! wait, never zero times — and an effect told from another thread must come
//! out in exactly one batch. Run with `--cfg sans_effort_loom` (`test:loom`), which swaps
//! the driver's lock and wake flag for loom's.

#![cfg(sans_effort_loom)]

use loom::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};
use sans_effort_core::{
    driver::{Driver, outbox::Outbox, status::Status},
    reply::handle::ReplyHandle,
};
use std::{
    future::poll_fn,
    sync::PoisonError,
    task::{Poll, Waker},
};
use testresult::TestResult;

/// A message that may arrive from another thread, waking whoever waits.
/// Registers the waker, then looks again — as real channels do — so a send
/// between the first look and the registration is not missed.
#[derive(Default)]
struct Signal {
    sent: AtomicBool,
    waiter: Mutex<Option<Waker>>,
}

impl Signal {
    /// Send, and wake the waiter, if any. The waker stays registered, so
    /// every sender wakes it.
    fn send(&self) {
        self.sent.store(true, Ordering::Release);
        let waiter = self
            .waiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(waker) = waiter {
            waker.wake();
        }
    }

    async fn recv(&self) {
        poll_fn(|cx| {
            if self.sent.load(Ordering::Acquire) {
                return Poll::Ready(());
            }
            *self.waiter.lock().unwrap_or_else(PoisonError::into_inner) = Some(cx.waker().clone());
            if self.sent.load(Ordering::Acquire) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    }
}

/// A driver whose routine awaits each signal in turn, counting the signals
/// it has received.
struct Waiting {
    driver: Driver<()>,
    received: Arc<AtomicUsize>,
    /// How often the hook — the host's "this machine woke" — was called.
    told: Arc<AtomicUsize>,
}

impl Waiting {
    fn new(signals: &[Arc<Signal>]) -> Self {
        let signals = signals.to_vec();
        let received = Arc::new(AtomicUsize::new(0));
        let driver = Driver::new({
            let received = Arc::clone(&received);
            move |_: Outbox<()>| async move {
                for signal in signals {
                    signal.recv().await;
                    received.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        let told = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&told);
        driver.on_wake(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        Self {
            driver,
            received,
            told,
        }
    }

    fn received(&self) -> usize {
        self.received.load(Ordering::SeqCst)
    }

    fn told(&self) -> usize {
        self.told.load(Ordering::SeqCst)
    }
}

fn sending(signal: &Arc<Signal>) -> thread::JoinHandle<()> {
    let signal = Arc::clone(signal);
    thread::spawn(move || signal.send())
}

fn joined(thread: thread::JoinHandle<()>) -> TestResult {
    thread.join().map_err(|_| "a model thread panicked")?;
    Ok(())
}

/// A message sent while the driver polls: either the poll sees it, or the
/// hook is told — and a resume then receives it. Then the same for a second
/// wait, which only a flag cleared at the start of each poll can announce: a
/// stale flag from the first wake would swallow the second.
#[test]
fn a_wake_racing_a_poll_is_never_lost() {
    loom::model(|| {
        let Ok(()) = wake_racing_a_poll();
    });
}

fn wake_racing_a_poll() -> TestResult {
    let (first, second) = (Arc::new(Signal::default()), Arc::new(Signal::default()));
    let mut waiting = Waiting::new(&[Arc::clone(&first), Arc::clone(&second)]);

    let sender = sending(&first);
    drop(waiting.driver.resume());
    joined(sender)?;
    if waiting.received() == 0 {
        assert_eq!(waiting.told(), 1, "the first wake reached the hook");
        drop(waiting.driver.resume());
    }
    assert_eq!(waiting.received(), 1);

    let told = waiting.told();
    let sender = sending(&second);
    drop(waiting.driver.resume());
    joined(sender)?;
    if waiting.received() == 1 {
        assert_eq!(
            waiting.told(),
            told + 1,
            "the second wake reached the hook too"
        );
        drop(waiting.driver.resume());
    }
    assert_eq!(waiting.driver.status(), Status::Complete);
    Ok(())
}

/// Two wakes racing each other while the driver is idle tell the hook once:
/// the host learns "this machine can run", not how many times.
#[test]
fn concurrent_wakes_tell_the_hook_once_per_wait() {
    loom::model(|| {
        let Ok(()) = concurrent_wakes();
    });
}

fn concurrent_wakes() -> TestResult {
    let signal = Arc::new(Signal::default());
    let mut waiting = Waiting::new(&[Arc::clone(&signal)]);
    drop(waiting.driver.resume());
    assert_eq!(waiting.driver.status(), Status::Idle);

    let senders = [sending(&signal), sending(&signal)];
    for sender in senders {
        joined(sender)?;
    }
    assert_eq!(waiting.told(), 1);

    drop(waiting.driver.resume());
    assert_eq!(waiting.driver.status(), Status::Complete);
    Ok(())
}

/// A routine hands a clone of its outbox to another thread, which tells an
/// effect while the driver polls (spuriously) and then signals. Every effect
/// comes out in exactly one batch: none lost between a tell and a drain,
/// none drained twice.
#[test]
fn an_effect_told_from_another_thread_comes_out_once() {
    loom::model(|| {
        let Ok(()) = effect_told_from_another_thread();
    });
}

fn effect_told_from_another_thread() -> TestResult {
    let signal = Arc::new(Signal::default());
    let handed: Arc<Mutex<Option<Outbox<u8>>>> = Arc::default();
    let mut driver = Driver::new({
        let (signal, handed) = (Arc::clone(&signal), Arc::clone(&handed));
        move |outbox: Outbox<u8>| async move {
            *handed.lock().unwrap_or_else(PoisonError::into_inner) = Some(outbox.clone());
            outbox.tell(0);
            signal.recv().await;
        }
    });

    let (mut told, _) = driver.resume().into_parts();
    let outbox = handed
        .lock()?
        .take()
        .ok_or("the routine handed its outbox over")?;
    let teller = thread::spawn({
        let signal = Arc::clone(&signal);
        move || {
            outbox.tell(1);
            signal.send();
        }
    });
    told.extend(driver.resume().into_parts().0);
    joined(teller)?;
    while driver.status() != Status::Complete {
        told.extend(driver.resume().into_parts().0);
    }
    told.extend(driver.resume().into_parts().0);

    told.sort_unstable();
    assert_eq!(told, [0, 1]);
    Ok(())
}

/// A request: just the handle, whose id the host reads.
struct Request(ReplyHandle<()>);

/// Two threads each record a request on a clone of one outbox, while the
/// driver drains it. Whatever the interleaving, the host sees the ids in the
/// order they were minted: 1, then 2.
#[test]
fn requests_built_at_once_leave_in_the_order_they_were_numbered() {
    loom::model(|| {
        let Ok(()) = builders_at_once();
    });
}

fn builders_at_once() -> TestResult {
    let handed: Arc<Mutex<Option<Outbox<Request>>>> = Arc::default();
    let mut driver = Driver::new({
        let handed = Arc::clone(&handed);
        move |outbox: Outbox<Request>| async move {
            *handed.lock().unwrap_or_else(PoisonError::into_inner) = Some(outbox);
            core::future::pending::<()>().await;
        }
    });
    drop(driver.resume());
    let outbox = handed
        .lock()?
        .take()
        .ok_or("the routine handed its outbox over")?;

    let builders: Vec<_> = (0..2)
        .map(|_| {
            let outbox = outbox.clone();
            thread::spawn(move || outbox.ask(Request).into_future())
        })
        .collect();
    let mut seen: Vec<u64> = driver
        .resume()
        .into_iter()
        .map(|Request(reply)| reply.id())
        .collect();
    let mut kept = Vec::new();
    for builder in builders {
        kept.push(builder.join().map_err(|_| "a builder panicked")?);
    }
    seen.extend(driver.resume().into_iter().map(|Request(reply)| reply.id()));

    assert_eq!(seen, [1, 2]);
    drop(kept);
    Ok(())
}
