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
    time::Duration,
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
/// recorded it: reply to an ask — now, or after a delay on the run's virtual
/// clock — begin a spawned child, or, for a tell, nothing. Replies and a
/// child's first resume are queued, not run at once, so the schedule decides
/// when they happen.
///
/// # Errors
///
/// [`Stalled`] if some machine can never progress: nothing is queued,
/// nothing woke, no timer is set, and it has not completed.
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
) -> Result<Completed, Stalled> {
    drive_with(root, Fifo, handle)
}

/// As [`drive`], with `schedule` choosing, at every step, which ready action
/// runs next — an effect for the handler, a queued reply, a child's first
/// resume, a woken machine's resume — whether to resume some machine
/// spuriously first, and whether to kill one. A routine whose output depends
/// on the schedule, or that a spurious resume breaks, fails under one.
///
/// Each machine's effects reach the handler in the order it recorded them,
/// as a batch's effects reach any host; what the schedule varies is when
/// replies are delivered, when machines resume, and how machines interleave.
///
/// _Time is virtual._ A reply sent with [`Answers::reply_after`] waits on a
/// timer, and timers fire — in the order they fall due, the clock jumping to
/// each — only when nothing else is ready, as on tokio's paused clock. Work
/// that can happen now always happens before a timer, under every schedule.
///
/// _A killed machine_ is dropped, as a host's `free` or a panic would drop
/// it: its future and everything it held go, and a channel whose last sender
/// it held closes. Effects it recorded before it died still reach the
/// handler, as they would a host that already had them; replies to it and
/// resumes of it are discarded. It is not counted as stalled.
///
/// # Errors
///
/// As [`drive`].
pub fn drive_with<E: 'static, S: Schedule, H: FnMut(E, &mut Answers<'_, E>)>(
    root: Driver<E>,
    mut schedule: S,
    mut handle: H,
) -> Result<Completed, Stalled> {
    let mut world = World {
        machines: Vec::new(),
        ready: Vec::new(),
        timers: Vec::new(),
        now: Duration::ZERO,
        woken: Arc::new(Mutex::new(VecDeque::new())),
    };
    world.adopt(Machine::Migrating(root));
    let mut killed = Vec::new();

    loop {
        for at in take_woken(&world.woken) {
            if !world
                .ready
                .iter()
                .any(|a| matches!(a, Action::Resume(m) if *m == at))
            {
                world.ready.push(Action::Resume(at));
            }
        }

        if let Some(at) = schedule.kill(world.machines.len())
            && world.kill(at)
        {
            killed.push(at);
            continue;
        }

        if let Some(at) = schedule.stutter(world.machines.len())
            && let Some(machine) = world.machine(at)
        {
            let step = machine.resume();
            world.push_effects(at, step);
            continue;
        }

        if world.ready.is_empty() && !world.fire_next_timer() {
            let stuck: Vec<usize> = world
                .machines
                .iter()
                .enumerate()
                .filter(|(_, m)| m.as_ref().is_some_and(|m| m.status() != Status::Complete))
                .map(|(at, _)| at)
                .collect();
            return if stuck.is_empty() {
                Ok(Completed { killed })
            } else {
                Err(Stalled { machines: stuck })
            };
        }

        let offered = eligible(&world.ready);
        // A choice past the end means the newest, as `Schedule::next` says.
        let choice = schedule.next(offered.len());
        let pick = offered
            .get(choice)
            .or(offered.last())
            .copied()
            .unwrap_or_default();
        match world.ready.remove(pick) {
            Action::Effect(at, effect) => {
                let mut answers = Answers {
                    at,
                    world: &mut world,
                };
                handle(effect, &mut answers);
            }
            Action::Reply(at, deliver) => {
                if let Some(machine) = world.machine(at) {
                    let step = deliver(machine);
                    world.push_effects(at, step);
                }
            }
            Action::Resume(at) => {
                if let Some(machine) = world.machine(at) {
                    let step = machine.resume();
                    world.push_effects(at, step);
                }
            }
        }
    }
}

/// What a handler can do with an effect: reply to it, now or later, or begin
/// a child the effect carried. Named for the machine that recorded the
/// effect.
pub struct Answers<'a, E> {
    at: usize,
    world: &'a mut World<E>,
}

impl<E: 'static> Answers<'_, E> {
    /// Queue `answer` for the ask `reply` names; the schedule decides when it
    /// is delivered.
    pub fn reply<A: Answer + 'static>(&mut self, reply: ReplyHandle<A>, answer: A) {
        self.world
            .ready
            .push(Action::Reply(self.at, deliver(reply, answer)));
    }

    /// Queue `answer` for the ask `reply` names once `delay` has passed on
    /// the run's virtual clock: a sleep, or a slow host. It is delivered no
    /// sooner than every action that is ready before then.
    pub fn reply_after<A: Answer + 'static>(
        &mut self,
        delay: Duration,
        reply: ReplyHandle<A>,
        answer: A,
    ) {
        self.world.timers.push(Timer {
            due: self.world.now.saturating_add(delay),
            at: self.at,
            deliver: deliver(reply, answer),
        });
    }

    /// Begin a child whose future is `Send`: it joins the machines, and its
    /// first resume is queued.
    pub fn spawn<M: FnOnce(Outbox<E>) -> BoxedRoutine>(&mut self, make: M) {
        self.world
            .adopt(Machine::Migrating(Driver::from_boxed(make)));
    }

    /// Begin a child whose future need not be `Send`, as [`spawn`](Self::spawn)
    /// does; every machine here runs on this thread.
    pub fn spawn_pinned<M: FnOnce(Outbox<E>) -> LocalBoxedRoutine>(&mut self, make: M) {
        self.world.adopt(Machine::Local(Driver::local_boxed(make)));
    }

    /// Which machine recorded the effect: the root is `0`, then each child in
    /// the order it was spawned.
    #[must_use]
    pub const fn machine(&self) -> usize {
        self.at
    }

    /// The run's virtual clock: how long since it began, which is the due
    /// time of the last timer to fire.
    #[must_use]
    pub const fn now(&self) -> Duration {
        self.world.now
    }
}

impl<E> core::fmt::Debug for Answers<'_, E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Answers")
            .field("machine", &self.at)
            .field("now", &self.world.now)
            .finish_non_exhaustive()
    }
}

/// A run that did not stall: every machine still running completed.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Completed {
    killed: Vec<usize>,
}

impl Completed {
    /// The machines the schedule killed before they completed, by index, in
    /// the order it killed them.
    #[must_use]
    pub fn killed(&self) -> &[usize] {
        &self.killed
    }
}

/// Everything a run keeps between steps.
struct World<E> {
    /// By index; `None` once killed.
    machines: Vec<Option<Machine<E>>>,
    ready: Vec<Action<E>>,
    /// In the order they were set, which breaks ties between equal due times.
    timers: Vec<Timer<E>>,
    now: Duration,
    woken: Woken,
}

impl<E> World<E> {
    /// The machine at `at`, unless it was killed.
    fn machine(&mut self, at: usize) -> Option<&mut Machine<E>> {
        self.machines.get_mut(at).and_then(Option::as_mut)
    }

    /// Add a machine: report its wakes by its index, and queue its first
    /// resume.
    fn adopt(&mut self, machine: Machine<E>) {
        let at = self.machines.len();
        let woken = Arc::clone(&self.woken);
        machine.on_wake(move || woken.lock().push_back(at));
        self.machines.push(Some(machine));
        self.ready.push(Action::Resume(at));
    }

    /// Drop the machine at `at` and its timers. Whether it was still running.
    fn kill(&mut self, at: usize) -> bool {
        let Some(machine) = self.machines.get_mut(at).and_then(Option::take) else {
            return false;
        };
        self.timers.retain(|timer| timer.at != at);
        machine.status() != Status::Complete
    }

    /// Fire the timer that falls due first — the earliest set, among equals —
    /// moving the clock to its due time and queueing its reply. Whether there
    /// was one.
    fn fire_next_timer(&mut self) -> bool {
        let next = self
            .timers
            .iter()
            .enumerate()
            .min_by_key(|(_, timer)| timer.due)
            .map(|(i, _)| i);
        let Some(next) = next else {
            return false;
        };
        let timer = self.timers.remove(next);
        self.now = self.now.max(timer.due);
        self.ready.push(Action::Reply(timer.at, timer.deliver));
        true
    }

    fn push_effects(&mut self, at: usize, step: Yield<E>) {
        self.ready
            .extend(step.into_iter().map(|effect| Action::Effect(at, effect)));
    }
}

/// A reply waiting on the virtual clock.
struct Timer<E> {
    due: Duration,
    at: usize,
    deliver: Deliver<E>,
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

fn deliver<E, A: Answer + 'static>(reply: ReplyHandle<A>, answer: A) -> Deliver<E> {
    Box::new(move |machine: &mut Machine<E>| machine.reply(reply, answer))
}

/// Which machines woke, in order.
type Woken = Arc<Mutex<VecDeque<usize>>>;

fn take_woken(woken: &Woken) -> Vec<usize> {
    woken.lock().drain(..).collect()
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
    #[should_panic(expected = "returned Pending")]
    fn pending_futures_are_reported() {
        run_now(core::future::pending::<()>());
    }

    mod driving {
        use super::super::{schedule::Choices, *};
        use crate::{
            join::join,
            select::{Either, select},
            step::Step,
        };
        use alloc::vec::Vec;
        use core::ops::ControlFlow;
        use testresult::TestResult;

        enum Effect {
            Ask(ReplyHandle<u64>),
            Say(u64),
            /// Sleep this many milliseconds.
            Sleep(u64, ReplyHandle<()>),
            Spawn(Box<dyn FnOnce(Outbox<Effect>) -> BoxedRoutine + Send>),
            SpawnPinned(Box<dyn FnOnce(Outbox<Effect>) -> LocalBoxedRoutine + Send>),
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

        /// Answers each ask with its request id — at once, or after
        /// `ask_delay` — and each sleep after its length; begins each child.
        /// What is said, with the clock when it was said, and how the run
        /// ended.
        fn run<S: Schedule>(
            root: Driver<Effect>,
            schedule: S,
            ask_delay: Option<Duration>,
        ) -> (Vec<(u64, Duration)>, Result<Completed, Stalled>) {
            let mut said = Vec::new();
            let ran = drive_with(root, schedule, |effect, host| match effect {
                Effect::Ask(reply) => {
                    let id = reply.id();
                    match ask_delay {
                        Some(delay) => host.reply_after(delay, reply, id),
                        None => host.reply(reply, id),
                    }
                }
                Effect::Say(n) => said.push((n, host.now())),
                Effect::Sleep(millis, reply) => {
                    host.reply_after(Duration::from_millis(millis), reply, ());
                }
                Effect::Spawn(make) => host.spawn(make),
                Effect::SpawnPinned(make) => host.spawn_pinned(make),
            });
            (said, ran)
        }

        /// What is said, in a run that must complete.
        fn said<S: Schedule>(root: Driver<Effect>, schedule: S) -> TestResult<Vec<u64>> {
            let (said, ran) = run(root, schedule, None);
            ran?;
            Ok(said.into_iter().map(|(n, _)| n).collect())
        }

        const fn ms(millis: u64) -> Duration {
            Duration::from_millis(millis)
        }

        #[test]
        fn a_join_says_the_same_under_every_schedule() {
            bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
                let root = Driver::new(|outbox| Pair(outbox).run());
                let Ok(said) = said(root, Choices::new(bytes.iter().copied()));
                assert_eq!(said, [1, 2]);
            });
        }

        /// The schedule does reorder replies: a routine that reports
        /// arrival order says something else under some choices.
        #[test]
        fn a_race_shows_the_schedule() -> TestResult {
            let fifo = said(Driver::new(|outbox| Racer(outbox).run()), Fifo)?;
            assert_eq!(fifo, [1, 2]);
            // Every ten-byte sequence of 1s and 2s: neither ever stutters, and
            // with two actions ready they pick the newer or the older.
            let reordered = (0..1_u32 << 10).any(|bits| {
                let bytes = (0..10).map(move |i| if bits >> i & 1 == 1 { 1 } else { 2 });
                let root = Driver::new(|outbox| Racer(outbox).run());
                let Ok(said) = said(root, Choices::new(bytes));
                said == [2, 1]
            });
            assert!(reordered, "some choice delivers the second reply first");
            Ok(())
        }

        /// Sleeps for both lengths at once, saying each as it wakes.
        struct Sleepers(Outbox<Effect>, [u64; 2]);

        impl Step for Sleepers {
            async fn step(&mut self) -> ControlFlow<()> {
                let nap = |millis: u64| {
                    let outbox = self.0.clone();
                    async move {
                        outbox.ask(|reply| Effect::Sleep(millis, reply)).await;
                        outbox.tell(Effect::Say(millis));
                    }
                };
                let [a, b] = self.1;
                join(nap(a), nap(b)).await;
                ControlFlow::Break(())
            }
        }

        /// Timers fire in due order, whatever order they were set in and
        /// whatever the schedule, and the clock reads each due time.
        #[test]
        fn timers_fire_in_due_order_under_every_schedule() {
            bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
                let root = Driver::new(|outbox| Sleepers(outbox, [30, 10]).run());
                let (said, ran) = run(root, Choices::new(bytes.iter().copied()), None);
                assert_eq!(said, [(10, ms(10)), (30, ms(30))]);
                assert_eq!(ran, Ok(Completed::default()));
            });
        }

        /// Takes an answer or a 10 ms timeout, whichever comes first; says
        /// the answer, or 0 for the timeout.
        struct Deadline(Outbox<Effect>);

        impl Step for Deadline {
            async fn step(&mut self) -> ControlFlow<()> {
                let answer = self.0.ask(Effect::Ask);
                let timeout = self.0.ask(|reply| Effect::Sleep(10, reply));
                let said = match select(answer, timeout).await {
                    Either::Left(n) => n,
                    Either::Right(()) => 0,
                };
                self.0.tell(Effect::Say(said));
                ControlFlow::Break(())
            }
        }

        /// An answer ready now beats any timer; a slow one beats the timeout
        /// if it is due first, or at the same time — set first, it fires
        /// first — and loses after. The same under every schedule.
        #[test]
        fn ready_work_beats_a_timer_and_a_slow_answer_races_it_in_due_order() {
            bolero::check!()
                .with_type::<(Option<u8>, Vec<u8>)>()
                .for_each(|(delay, bytes)| {
                    let root = Driver::new(|outbox| Deadline(outbox).run());
                    let delay = delay.map(|d| ms(d.into()));
                    let (said, ran) = run(root, Choices::new(bytes.iter().copied()), delay);
                    let expected = match delay {
                        None => (1, Duration::ZERO),
                        Some(d) if d <= ms(10) => (1, d),
                        Some(_) => (0, ms(10)),
                    };
                    assert_eq!(said, [expected]);
                    assert_eq!(ran, Ok(Completed::default()));
                });
        }

        /// Spawns a child that asks, then sends its answer to the parent
        /// over a channel; says what arrives, or 0 if the channel closes.
        struct Parent(Outbox<Effect>);

        impl Step for Parent {
            async fn step(&mut self) -> ControlFlow<()> {
                let (tx, rx) = async_channel::bounded(1);
                self.0
                    .tell(Effect::Spawn(Box::new(move |outbox: Outbox<Effect>| {
                        Box::pin(async move {
                            let n = outbox.ask(Effect::Ask).await;
                            // The parent may be gone: nothing to tell.
                            tx.send(n).await.unwrap_or_default();
                        }) as BoxedRoutine
                    })));
                let n = rx.recv().await.unwrap_or(0);
                self.0.tell(Effect::Say(n));
                ControlFlow::Break(())
            }
        }

        /// Kills one machine as soon as it exists; otherwise `Fifo`.
        struct KillsOnce {
            target: usize,
            done: bool,
        }

        impl KillsOnce {
            const fn new(target: usize) -> Self {
                Self {
                    target,
                    done: false,
                }
            }
        }

        impl Schedule for KillsOnce {
            fn next(&mut self, _: usize) -> usize {
                0
            }

            fn stutter(&mut self, _: usize) -> Option<usize> {
                None
            }

            fn kill(&mut self, machines: usize) -> Option<usize> {
                (!self.done && machines > self.target).then(|| {
                    self.done = true;
                    self.target
                })
            }
        }

        /// Killing the child drops the sender it held: the parent's receive
        /// sees the channel closed, and the run completes, naming the kill.
        #[test]
        fn a_killed_machine_closes_its_channels() -> TestResult {
            let parent = || Driver::new(|outbox| Parent(outbox).run());
            assert_eq!(said(parent(), Fifo)?, [1]);

            let (said, ran) = run(parent(), KillsOnce::new(1), None);
            assert_eq!(said, [(0, Duration::ZERO)]);
            assert_eq!(ran?.killed(), [1], "the parent completes, naming the kill");
            Ok(())
        }

        /// Whatever is killed, and whenever, no survivor stalls; with no
        /// kill, the run is the usual one.
        #[test]
        fn survivors_complete_whatever_is_killed() {
            bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
                let Ok(()) = survivors_complete(bytes);
            });
        }

        fn survivors_complete(bytes: &[u8]) -> TestResult {
            let root = Driver::new(|outbox| Parent(outbox).run());
            let schedule = Choices::new(bytes.iter().copied()).crashing();
            let (said, ran) = run(root, schedule, None);
            if ran?.killed().is_empty() {
                assert_eq!(said, [(1, Duration::ZERO)]);
            }
            Ok(())
        }

        /// Spawns a child, and a pinned child holding an `Rc` (so its future
        /// is not `Send`); each says its number, as does the root.
        struct Family(Outbox<Effect>);

        impl Step for Family {
            async fn step(&mut self) -> ControlFlow<()> {
                self.0
                    .tell(Effect::Spawn(Box::new(|outbox: Outbox<Effect>| {
                        Box::pin(async move { outbox.tell(Effect::Say(10)) }) as BoxedRoutine
                    })));
                self.0
                    .tell(Effect::SpawnPinned(Box::new(|outbox: Outbox<Effect>| {
                        let local = alloc::rc::Rc::new(20);
                        Box::pin(async move { outbox.tell(Effect::Say(*local)) })
                            as LocalBoxedRoutine
                    })));
                self.0.tell(Effect::Say(0));
                ControlFlow::Break(())
            }
        }

        /// What each machine said, by machine, under `schedule`.
        fn family<S: Schedule>(schedule: S) -> TestResult<(Vec<(usize, u64)>, Completed)> {
            let mut said = Vec::new();
            let ran = drive_with(
                Driver::new(|outbox| Family(outbox).run()),
                schedule,
                |effect, host| match effect {
                    Effect::Say(n) => said.push((host.machine(), n)),
                    Effect::Spawn(make) => host.spawn(make),
                    Effect::SpawnPinned(make) => host.spawn_pinned(make),
                    Effect::Ask(_) | Effect::Sleep(..) => unreachable!("the family only tells"),
                },
            )?;
            said.sort_unstable();
            Ok((said, ran))
        }

        /// The root is machine 0, then each child in the order spawned; a
        /// pinned child runs as a local machine.
        #[test]
        fn children_are_numbered_in_spawn_order_and_pinned_ones_run() -> TestResult {
            let (said, ran) = family(Fifo)?;
            assert_eq!(said, [(0, 0), (1, 10), (2, 20)]);
            assert_eq!(ran.killed(), []);
            Ok(())
        }

        #[test]
        fn a_kill_is_reported_by_the_machine_it_named() -> TestResult {
            let (said, ran) = family(KillsOnce::new(2))?;
            assert_eq!(said, [(0, 0), (1, 10)], "the pinned child never ran");
            assert_eq!(ran.killed(), [2]);
            Ok(())
        }

        /// Takes the newest action, by naming one past the end.
        struct PastTheEnd;

        impl Schedule for PastTheEnd {
            fn next(&mut self, _: usize) -> usize {
                usize::MAX
            }

            fn stutter(&mut self, _: usize) -> Option<usize> {
                None
            }
        }

        /// Takes the newest action, by naming it.
        struct Newest;

        impl Schedule for Newest {
            fn next(&mut self, ready: usize) -> usize {
                ready - 1
            }

            fn stutter(&mut self, _: usize) -> Option<usize> {
                None
            }
        }

        #[test]
        fn a_choice_past_the_end_means_the_newest() -> TestResult {
            let racer = || Driver::new(|outbox| Racer(outbox).run());
            assert_eq!(said(racer(), PastTheEnd)?, said(racer(), Newest)?);
            Ok(())
        }

        /// Killing drops the machine's timers and no others, and says
        /// whether it was still running.
        #[test]
        fn a_kill_drops_only_its_own_timers() -> TestResult {
            let mut world: World<Effect> = World {
                machines: Vec::new(),
                ready: Vec::new(),
                timers: Vec::new(),
                now: Duration::ZERO,
                woken: Arc::new(Mutex::new(VecDeque::new())),
            };
            world.adopt(Machine::Migrating(Driver::new(|_: Outbox<Effect>| {
                core::future::pending::<()>()
            })));
            world.adopt(Machine::Migrating(
                Driver::new(|_: Outbox<Effect>| async {}),
            ));
            drop(world.machine(1).ok_or("machine 1 is alive")?.resume());
            for at in [0, 1] {
                world.timers.push(Timer {
                    due: ms(5),
                    at,
                    deliver: Box::new(|_| Yield::new(Vec::new(), Vec::new())),
                });
            }

            assert!(world.kill(0), "it was waiting");
            let timers: Vec<usize> = world.timers.iter().map(|timer| timer.at).collect();
            assert_eq!(timers, [1], "only its own timer went");
            assert!(!world.kill(1), "it had completed");
            assert!(!world.kill(0), "already gone");
            Ok(())
        }

        /// The root completes; its child waits on a channel it holds the
        /// sender of, forever — and is named.
        #[test]
        fn a_stalled_child_is_named() -> TestResult {
            let root = Driver::new(|outbox: Outbox<Effect>| async move {
                outbox.tell(Effect::Spawn(Box::new(|_: Outbox<Effect>| {
                    Box::pin(async move {
                        let (tx, rx) = async_channel::bounded::<()>(1);
                        let _kept = tx;
                        rx.recv().await.unwrap_or_default();
                    }) as BoxedRoutine
                })));
            });
            let stalled = drive(root, |effect, host| {
                if let Effect::Spawn(make) = effect {
                    host.spawn(make);
                }
            })
            .err()
            .ok_or("the child waits forever")?;
            assert_eq!(stalled.machines(), [1]);
            Ok(())
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
        fn a_deadlock_is_reported_not_hung_on() -> TestResult {
            let (sender, receiver) = async_channel::unbounded();
            let root = Driver::new(move |_: Outbox<Effect>| {
                Stuck {
                    _sender: sender,
                    receiver,
                }
                .run()
            });
            let stalled = drive(root, |_, _| {}).err().ok_or("nothing can wake it")?;
            assert_eq!(stalled.machines(), [0]);
            Ok(())
        }
    }
}
