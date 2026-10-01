//! The workloads both harnesses measure, each written once as a plain
//! function: the routine, its driver, and the smallest host loop that runs
//! it to completion.
//!
//! - [`asks`]: `n` asks, each answered at once — one request's round trip
//!   through a driver, `n` times.
//! - [`fanout`]: `n` rounds of two asks in flight at once, answered in turn.
//! - [`ring`]: `nodes` drivers passing a counter around a ring of channels,
//!   `rounds` times; each hop is a message no effect reports, found by
//!   resuming whichever driver's wake hook fired.

use core::ops::ControlFlow;
use sans_effort_core::{
    driver::{Driver, outbox::Outbox},
    join::join,
    reply::handle::ReplyHandle,
    step::Step,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
};

/// The one effect the asking workloads use: a request for a `u64`.
pub(crate) enum Effect {
    Ask(ReplyHandle<u64>),
}

/// Asks `left` times, one at a time, summing the answers.
struct Asker {
    outbox: Outbox<Effect>,
    left: u64,
    sum: u64,
}

impl Step for Asker {
    async fn step(&mut self) -> ControlFlow<()> {
        if self.left == 0 {
            return ControlFlow::Break(());
        }
        self.left -= 1;
        self.sum = self.sum.wrapping_add(self.outbox.ask(Effect::Ask).await);
        ControlFlow::Continue(())
    }
}

/// `n` asks through one driver, each answered as soon as it appears. Returns
/// the number answered.
pub(crate) fn asks(n: u64) -> u64 {
    let mut driver = Driver::new(move |outbox| {
        Asker {
            outbox,
            left: n,
            sum: 0,
        }
        .run()
    });
    answer_all(&mut driver)
}

/// Two asks at a time, `left` rounds, summing the answers.
struct Fanner {
    outbox: Outbox<Effect>,
    left: u64,
    sum: u64,
}

impl Step for Fanner {
    async fn step(&mut self) -> ControlFlow<()> {
        if self.left == 0 {
            return ControlFlow::Break(());
        }
        self.left -= 1;
        let (a, b) = join(self.outbox.ask(Effect::Ask), self.outbox.ask(Effect::Ask)).await;
        self.sum = self.sum.wrapping_add(a).wrapping_add(b);
        ControlFlow::Continue(())
    }
}

/// `n` rounds of two asks in flight, answered in the order they appear.
/// Returns the number answered: `2 × n`.
pub(crate) fn fanout(n: u64) -> u64 {
    let mut driver = Driver::new(move |outbox| {
        Fanner {
            outbox,
            left: n,
            sum: 0,
        }
        .run()
    });
    answer_all(&mut driver)
}

/// Answer every ask the driver yields, in order, until it completes.
fn answer_all(driver: &mut Driver<Effect>) -> u64 {
    let mut answered = 0;
    let mut queue: VecDeque<Effect> = driver.resume().into();
    while let Some(Effect::Ask(reply)) = queue.pop_front() {
        answered += 1;
        queue.extend(driver.reply(reply, answered));
    }
    answered
}

/// One node of the ring: receive the counter, pass it on, `rounds` times.
/// The first node also starts the counter, and does not pass it on the last
/// time round.
struct Node {
    inbox: async_channel::Receiver<u64>,
    next: async_channel::Sender<u64>,
    first: bool,
    rounds: u64,
    started: bool,
}

impl Step for Node {
    async fn step(&mut self) -> ControlFlow<()> {
        if self.first && !self.started {
            self.started = true;
            // Unbounded, and the next node outlives the counter's travels.
            let _sent = self.next.try_send(1);
        }
        if self.rounds == 0 {
            return ControlFlow::Break(());
        }
        let Ok(count) = self.inbox.recv().await else {
            return ControlFlow::Break(());
        };
        self.rounds -= 1;
        if !(self.first && self.rounds == 0) {
            let _sent = self.next.try_send(count + 1);
        }
        ControlFlow::Continue(())
    }
}

/// Which drivers woke, in order, by index.
type Woken = Arc<Mutex<VecDeque<usize>>>;

/// `nodes` drivers passing a counter `rounds` times around a ring of
/// channels: `nodes × rounds` hops. Each driver's wake hook notes its index,
/// and the host resumes drivers in the order they woke. Returns the hops.
pub(crate) fn ring(nodes: usize, rounds: u64) -> u64 {
    let (senders, receivers): (Vec<_>, Vec<_>) =
        (0..nodes).map(|_| async_channel::unbounded()).unzip();
    let woken = Woken::default();

    // Node `at` sends to node `at + 1`, the last to the first.
    let nexts = senders.iter().cycle().skip(1).take(nodes).cloned();
    let mut drivers: Vec<Driver<()>> = receivers
        .into_iter()
        .zip(nexts)
        .enumerate()
        .map(|(at, (inbox, next))| {
            let driver = Driver::new(move |_outbox| {
                Node {
                    inbox,
                    next,
                    first: at == 0,
                    rounds,
                    started: false,
                }
                .run()
            });
            let woken = Woken::clone(&woken);
            driver.on_wake(move || {
                woken
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_back(at);
            });
            driver
        })
        .collect();
    drop(senders);

    for driver in &mut drivers {
        drop(driver.resume());
    }
    let mut hops = 0;
    loop {
        let next = woken
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(at) = next else { break };
        if let Some(driver) = drivers.get_mut(at) {
            drop(driver.resume());
            hops += 1;
        }
    }
    hops
}
