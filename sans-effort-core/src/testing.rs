//! Helpers for testing routines (feature `testing`): run one against a context whose every
//! future is ready at once, poll a single wait exactly once, or drive a
//! routine and everything it spawns through drivers, answering each effect
//! as it appears, in an order a [`Schedule`] picks.
//!
//! The cheapest test of a routine needs no driver and no runtime: a test
//! double implements the routine's traits, each call returns a ready future,
//! and one poll runs the whole routine to completion. The transcript of
//! calls the double recorded is the assertion.
//!
//! ```
//! use core::{cell::RefCell, ops::ControlFlow};
//! use sans_effort_core::{step::Step, testing::run_now};
//!
//! trait Console {
//!     async fn read_line(&self) -> String;
//!     fn write_line(&self, line: String);
//! }
//!
//! struct Greeter<C: Console>(C);
//!
//! impl<C: Console> Step for Greeter<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         let name = self.0.read_line().await;
//!         self.0.write_line(format!("Hello, {name}!"));
//!         ControlFlow::Break(())
//!     }
//! }
//!
//! // A mock: scripted input, recorded output, every future ready.
//! struct Mock(RefCell<Vec<String>>);
//!
//! impl Console for &Mock {
//!     async fn read_line(&self) -> String { "bob".into() }
//!     fn write_line(&self, line: String) { self.0.borrow_mut().push(line); }
//! }
//!
//! let mock = Mock(RefCell::new(Vec::new()));
//! run_now(Greeter(&mock).run());
//! assert_eq!(mock.0.borrow().as_slice(), ["Hello, bob!"]);
//! ```

pub mod schedule;

use self::schedule::{Fifo, Schedule};
use crate::{
    driver::{
        BoxedRoutine, Driver, LocalBoxedRoutine, Yield,
        outbox::Outbox,
        status::Status,
        sync::{Arc, Mutex},
    },
    reply::{Answer, handle::ReplyHandle},
};
use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use core::{
    future::{Future, IntoFuture, poll_fn},
    pin::{Pin, pin},
    task::{Context, Poll, Waker},
};

/// Poll `future` once with a no-op waker and return its output.
///
/// This is not an executor. It is for futures that are ready on the first
/// poll — a routine against a mock whose every method returns immediately —
/// and it says so loudly when that assumption fails.
///
/// # Panics
///
/// If the future returns `Pending`. With no waker, nothing could ever wake
/// it; the cause is a wait that needs a real runtime (a `tokio::time::sleep`,
/// a channel) reaching a test that meant to mock it.
#[expect(
    clippy::panic,
    reason = "a Pending future here is a test bug, and a bare panic is the clearest report"
)]
pub fn run_now<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);

    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!(
            "run_now: the future returned Pending. Every wait in a routine under test \
             must resolve at once; a native future (a sleep, a channel) is leaking into \
             a test context, or the routine awaited something no context provides."
        ),
    }
}

/// Turn `future` into a future, poll it exactly once, and hand it back,
/// whatever the poll returned.
///
/// Under a driver, turning an [`Asking`](crate::driver::asking::Asking) into a future
/// records its effect; this is how a test makes a request record itself and
/// then abandons it, the way the losing branch of a
/// [`select`](crate::select::select) is abandoned.
///
/// ```
/// use core::ops::ControlFlow;
/// use sans_effort_core::{
///     driver::{Driver, outbox::Outbox},
///     reply::handle::ReplyHandle,
///     step::Step,
///     testing::poll_once,
/// };
///
/// enum Effect {
///     Ask(ReplyHandle<String>),
/// }
///
/// struct GivesUp(Outbox<Effect>);
///
/// impl Step for GivesUp {
///     async fn step(&mut self) -> ControlFlow<()> {
///         // Record the request, then drop it without waiting for a reply.
///         drop(poll_once(self.0.ask(Effect::Ask)).await);
///         ControlFlow::Break(())
///     }
/// }
///
/// let mut driver = Driver::new(|outbox| GivesUp(outbox).run());
/// let step = driver.resume();
/// assert_eq!(step.effects().len(), 1, "the request was recorded");
/// assert_eq!(step.closed(), [1], "and abandoned");
/// ```
pub async fn poll_once<F: IntoFuture<IntoFuture: Unpin>>(future: F) -> F::IntoFuture {
    let mut future = future.into_future();
    poll_fn(|cx| {
        drop(Pin::new(&mut future).poll(cx));
        Poll::Ready(())
    })
    .await;
    future
}

/// Run `root` and every machine it spawns on this thread, answering each
/// effect through `handle`, in the order effects appear, until nothing can
/// progress. [`drive_with`] under [`Fifo`].
///
/// The handler sees each effect with an [`Answers`] for the machine that
/// recorded it: reply to an ask, begin a spawned child, or — for a tell —
/// nothing. Replies and a child's first resume are queued, not run at once,
/// so the schedule decides when they happen.
///
/// # Errors
///
/// [`Stalled`] if some machine can never progress: nothing is queued,
/// nothing woke, and it has not completed.
///
/// ```
/// use core::ops::ControlFlow;
/// use sans_effort_core::{
///     driver::{Driver, outbox::Outbox},
///     reply::handle::ReplyHandle,
///     step::Step,
///     testing::drive,
/// };
///
/// enum Effect {
///     Ask(ReplyHandle<u64>),
///     Say(u64),
/// }
///
/// struct Doubler(Outbox<Effect>);
///
/// impl Step for Doubler {
///     async fn step(&mut self) -> ControlFlow<()> {
///         let n = self.0.ask(Effect::Ask).await;
///         self.0.tell(Effect::Say(2 * n));
///         ControlFlow::Break(())
///     }
/// }
///
/// let mut said = Vec::new();
/// drive(Driver::new(|outbox| Doubler(outbox).run()), |effect, host| match effect {
///     Effect::Ask(reply) => host.reply(reply, 21),
///     Effect::Say(n) => said.push(n),
/// })
/// .expect("completes");
/// assert_eq!(said, [42]);
/// ```
pub fn drive<E: 'static, H: FnMut(E, &mut Answers<'_, E>)>(
    root: Driver<E>,
    handle: H,
) -> Result<(), Stalled> {
    drive_with(root, Fifo, handle)
}

/// As [`drive`], with `schedule` choosing, at every step, which ready action
/// runs next — an effect for the handler, a queued reply, a child's first
/// resume, a woken machine's resume — and whether to resume some machine
/// spuriously first. A routine whose output depends on the schedule, or that
/// a spurious resume breaks, fails under one.
///
/// Each machine's effects reach the handler in the order it recorded them,
/// as a batch's effects reach any host; what the schedule varies is when
/// replies are delivered, when machines resume, and how machines interleave.
///
/// # Errors
///
/// As [`drive`].
pub fn drive_with<E: 'static, S: Schedule, H: FnMut(E, &mut Answers<'_, E>)>(
    root: Driver<E>,
    mut schedule: S,
    mut handle: H,
) -> Result<(), Stalled> {
    let woken: Woken = Arc::new(Mutex::new(VecDeque::new()));
    let mut machines: Vec<Machine<E>> = Vec::new();
    let mut ready: Vec<Action<E>> = Vec::new();
    adopt(&mut machines, &mut ready, &woken, Machine::Migrating(root));

    loop {
        for at in take_woken(&woken) {
            if !ready
                .iter()
                .any(|a| matches!(a, Action::Resume(m) if *m == at))
            {
                ready.push(Action::Resume(at));
            }
        }

        if let Some(at) = schedule.stutter(machines.len())
            && let Some(machine) = machines.get_mut(at)
        {
            let step = machine.resume();
            push_effects(&mut ready, at, step);
            continue;
        }

        if ready.is_empty() {
            let stuck: Vec<usize> = machines
                .iter()
                .enumerate()
                .filter(|(_, m)| m.status() != Status::Complete)
                .map(|(at, _)| at)
                .collect();
            return if stuck.is_empty() {
                Ok(())
            } else {
                Err(Stalled { machines: stuck })
            };
        }

        let offered = eligible(&ready);
        let choice = schedule.next(offered.len()).min(offered.len() - 1);
        let pick = offered.get(choice).copied().unwrap_or_default();
        match ready.remove(pick) {
            Action::Effect(at, effect) => {
                let mut answers = Answers {
                    at,
                    machines: &mut machines,
                    ready: &mut ready,
                    woken: &woken,
                };
                handle(effect, &mut answers);
            }
            Action::Reply(at, deliver) => {
                if let Some(machine) = machines.get_mut(at) {
                    let step = deliver(machine);
                    push_effects(&mut ready, at, step);
                }
            }
            Action::Resume(at) => {
                if let Some(machine) = machines.get_mut(at) {
                    let step = machine.resume();
                    push_effects(&mut ready, at, step);
                }
            }
        }
    }
}

/// What a handler can do with an effect: reply to it, or begin a child the
/// effect carried. Named for the machine that recorded the effect.
pub struct Answers<'a, E> {
    at: usize,
    machines: &'a mut Vec<Machine<E>>,
    ready: &'a mut Vec<Action<E>>,
    woken: &'a Woken,
}

impl<E: 'static> Answers<'_, E> {
    /// Queue `answer` for the ask `reply` names; the schedule decides when it
    /// is delivered.
    pub fn reply<A: Answer + 'static>(&mut self, reply: ReplyHandle<A>, answer: A) {
        self.ready.push(Action::Reply(
            self.at,
            Box::new(move |machine: &mut Machine<E>| machine.reply(reply, answer)),
        ));
    }

    /// Begin a child whose future is `Send`: it joins the machines, and its
    /// first resume is queued.
    pub fn spawn<M: FnOnce(Outbox<E>) -> BoxedRoutine>(&mut self, make: M) {
        let child = Machine::Migrating(Driver::from_boxed(make));
        adopt(self.machines, self.ready, self.woken, child);
    }

    /// Begin a child whose future need not be `Send`, as [`spawn`](Self::spawn)
    /// does; every machine here runs on this thread.
    pub fn spawn_pinned<M: FnOnce(Outbox<E>) -> LocalBoxedRoutine>(&mut self, make: M) {
        let child = Machine::Local(Driver::local_boxed(make));
        adopt(self.machines, self.ready, self.woken, child);
    }

    /// Which machine recorded the effect: the root is `0`, then each child in
    /// the order it was spawned.
    #[must_use]
    pub const fn machine(&self) -> usize {
        self.at
    }
}

impl<E> core::fmt::Debug for Answers<'_, E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Answers")
            .field("machine", &self.at)
            .finish_non_exhaustive()
    }
}

/// A machine the runner drives: migrating or local. All run on one thread.
enum Machine<E> {
    Migrating(Driver<E>),
    Local(Driver<E, dyn Future<Output = ()>>),
}

impl<E> Machine<E> {
    fn resume(&mut self) -> Yield<E> {
        match self {
            Machine::Migrating(d) => d.resume(),
            Machine::Local(d) => d.resume(),
        }
    }

    fn reply<A: Answer>(&mut self, reply: ReplyHandle<A>, answer: A) -> Yield<E> {
        match self {
            Machine::Migrating(d) => d.reply(reply, answer),
            Machine::Local(d) => d.reply(reply, answer),
        }
    }

    const fn status(&self) -> Status {
        match self {
            Machine::Migrating(d) => d.status(),
            Machine::Local(d) => d.status(),
        }
    }

    fn on_wake<H: Fn() + Send + Sync + 'static>(&self, hook: H) {
        match self {
            Machine::Migrating(d) => d.on_wake(hook),
            Machine::Local(d) => d.on_wake(hook),
        }
    }
}

/// Something the runner can do next.
enum Action<E> {
    /// Show the handler an effect `machine` recorded.
    Effect(usize, E),
    /// Deliver a queued reply to `machine`.
    Reply(usize, Deliver<E>),
    /// Resume `machine`: its first resume, or because it woke.
    Resume(usize),
}

/// A queued reply, typed answer and all, waiting to be delivered.
type Deliver<E> = Box<dyn FnOnce(&mut Machine<E>) -> Yield<E>>;

/// Which machines woke, in order.
type Woken = Arc<Mutex<VecDeque<usize>>>;

fn take_woken(woken: &Woken) -> Vec<usize> {
    woken.lock().drain(..).collect()
}

/// Add a machine: report its wakes by its index, and queue its first resume.
fn adopt<E>(
    machines: &mut Vec<Machine<E>>,
    ready: &mut Vec<Action<E>>,
    woken: &Woken,
    machine: Machine<E>,
) {
    let at = machines.len();
    let woken = Arc::clone(woken);
    machine.on_wake(move || woken.lock().push_back(at));
    machines.push(machine);
    ready.push(Action::Resume(at));
}

/// The ready actions a schedule may pick, by index: everything but an effect
/// with an older one from the same machine still waiting.
fn eligible<E>(ready: &[Action<E>]) -> Vec<usize> {
    let mut seen: Vec<usize> = Vec::new();
    ready
        .iter()
        .enumerate()
        .filter_map(|(i, action)| match action {
            Action::Effect(at, _) if seen.contains(at) => None,
            Action::Effect(at, _) => {
                seen.push(*at);
                Some(i)
            }
            Action::Reply(..) | Action::Resume(_) => Some(i),
        })
        .collect()
}

fn push_effects<E>(ready: &mut Vec<Action<E>>, at: usize, step: Yield<E>) {
    ready.extend(step.into_iter().map(|effect| Action::Effect(at, effect)));
}

/// Some machines can never progress: nothing was queued, none had woken, and
/// they had not completed. A deadlock, or a routine waiting on something no
/// machine will provide.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("stalled: machines {machines:?} can never progress")]
pub struct Stalled {
    machines: Vec<usize>,
}

impl Stalled {
    /// The machines that had not completed, by index (the root is `0`).
    #[must_use]
    pub fn machines(&self) -> &[usize] {
        &self.machines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_futures_complete() {
        assert_eq!(run_now(async { 1 + 1 }), 2);
    }

    #[test]
    #[should_panic(expected = "returned Pending")]
    fn pending_futures_are_reported() {
        run_now(core::future::pending::<()>());
    }

    mod driving {
        #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

        use super::super::{schedule::Choices, *};
        use crate::{join::join, step::Step};
        use alloc::vec::Vec;
        use core::ops::ControlFlow;

        enum Effect {
            Ask(ReplyHandle<u64>),
            Say(u64),
        }

        /// Asks twice at once, then says the answers in the order it asked.
        struct Pair(Outbox<Effect>);

        impl Step for Pair {
            async fn step(&mut self) -> ControlFlow<()> {
                let (a, b) = join(self.0.ask(Effect::Ask), self.0.ask(Effect::Ask)).await;
                self.0.tell(Effect::Say(a));
                self.0.tell(Effect::Say(b));
                ControlFlow::Break(())
            }
        }

        /// Says each answer as it arrives: its output is the reply order.
        struct Racer(Outbox<Effect>);

        impl Step for Racer {
            async fn step(&mut self) -> ControlFlow<()> {
                let outbox = self.0.clone();
                let say = |n| outbox.tell(Effect::Say(n));
                let first = async {
                    say(self.0.ask(Effect::Ask).await);
                };
                let second = async {
                    say(self.0.ask(Effect::Ask).await);
                };
                join(first, second).await;
                ControlFlow::Break(())
            }
        }

        /// Answers each ask with its request id; records what is said.
        fn said<S: Schedule>(root: Driver<Effect>, schedule: S) -> Vec<u64> {
            let mut said = Vec::new();
            drive_with(root, schedule, |effect, host| match effect {
                Effect::Ask(reply) => {
                    let id = reply.id();
                    host.reply(reply, id);
                }
                Effect::Say(n) => said.push(n),
            })
            .expect("completes");
            said
        }

        #[test]
        fn a_join_says_the_same_under_every_schedule() {
            bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
                let root = Driver::new(|outbox| Pair(outbox).run());
                assert_eq!(said(root, Choices::new(bytes.iter().copied())), [1, 2]);
            });
        }

        /// The schedule does reorder replies: a routine that reports
        /// arrival order says something else under some choices.
        #[test]
        fn a_race_shows_the_schedule() {
            let fifo = said(Driver::new(|outbox| Racer(outbox).run()), Fifo);
            assert_eq!(fifo, [1, 2]);
            // Every ten-byte sequence of 1s and 2s: neither ever stutters, and
            // with two actions ready they pick the newer or the older.
            let reordered = (0..1_u32 << 10).any(|bits| {
                let bytes = (0..10).map(move |i| if bits >> i & 1 == 1 { 1 } else { 2 });
                let root = Driver::new(|outbox| Racer(outbox).run());
                said(root, Choices::new(bytes)) == [2, 1]
            });
            assert!(reordered, "some choice delivers the second reply first");
        }

        /// Waits on a channel whose sender it keeps and never uses.
        struct Stuck {
            _sender: async_channel::Sender<()>,
            receiver: async_channel::Receiver<()>,
        }

        impl Step for Stuck {
            async fn step(&mut self) -> ControlFlow<()> {
                let _never = self.receiver.recv().await;
                ControlFlow::Break(())
            }
        }

        #[test]
        fn a_deadlock_is_reported_not_hung_on() {
            let (sender, receiver) = async_channel::unbounded();
            let root = Driver::new(move |_: Outbox<Effect>| {
                Stuck {
                    _sender: sender,
                    receiver,
                }
                .run()
            });
            let stalled = drive(root, |_, _| {}).expect_err("nothing can wake it");
            assert_eq!(stalled.machines(), [0]);
        }
    }
}
