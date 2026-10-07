//! The host table under a real multi-threaded host.
//!
//! The spec's traces (`spec_traces.rs`) check the table call by call; loom
//! checks core's waker. Neither runs the table's own locks — the handle table,
//! each machine's lock, the per-thread wake collection — from many threads at
//! once. This host does: 2–4 threads share one queue of events, take whatever
//! is next, and make the call without coordinating. So two threads collide on
//! one machine (`BUSY`, retried), calls on different machines overlap, a
//! spurious resume lands anywhere, and replies go to requests already closed
//! (`STALE`). The workload is rings of machines passing a token over
//! channels, every hop asking the host for its request id back — a reply
//! routed to the wrong machine or request makes the routine panic.
//!
//! Each seed picks the thread count and the host's dice; thread timing does
//! the rest, so a seed reproduces choices, not interleavings.
//!
//! Wakes outside any call — from `free` or a panic — do not arise here; the
//! table's unit tests cover them. A table that drops `woke` frames stalls
//! this host on the first seed.

use async_channel::{Receiver, Sender};
use sans_effort_core::{
    boundary::{
        codec::{Encode, Reader, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
    driver::{BoxedRoutine, outbox::Outbox, status::Status},
    reply::handle::ReplyHandle,
    select::select,
};
use sans_effort_host::{
    contract::{FRAME_ASK, FRAME_CLOSED, FRAME_TELL, FRAME_WOKE},
    error::Error,
    table,
};
use std::{
    collections::{BTreeSet, HashSet, VecDeque},
    future::ready,
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};
use testresult::TestResult;

const RINGS: usize = 3;
const WORKERS: usize = 4;
const LAPS: usize = 5;
const SEEDS: u64 = 16;

enum Effect {
    Ask(ReplyHandle<u64>),
    Spawn(Box<dyn FnOnce(Outbox<Effect>) -> BoxedRoutine + Send>),
}

enum View {
    Ask(u64),
    Spawned(u64),
}

/// As a binding's vocabulary does: a child is registered when the call that
/// spawned it is encoded.
impl HostEffect for Effect {
    type View = View;

    fn split(self) -> (View, Option<Pending>) {
        match self {
            Effect::Ask(reply) => (View::Ask(reply.id()), Some(Pending::U64(reply))),
            Effect::Spawn(make) => (View::Spawned(table::new_boxed(make)), None),
        }
    }
}

impl Encode for View {
    fn encode(&self, w: &mut Writer) {
        let (tag, value) = match self {
            View::Ask(id) => (1, id),
            View::Spawned(handle) => (2, handle),
        };
        w.u8(tag);
        w.u64(*value);
    }
}

/// A ring's root: spawns its workers, starts the token, and waits until
/// every worker has finished.
fn ring(hops: Arc<AtomicUsize>) -> impl FnOnce(Outbox<Effect>) -> BoxedRoutine + Send {
    move |outbox| {
        Box::pin(async move {
            let (mut next, links): (Vec<Sender<()>>, Vec<Receiver<()>>) =
                (0..WORKERS).map(|_| async_channel::unbounded()).unzip();
            if let Some(first) = next.first() {
                assert!(first.try_send(()).is_ok(), "the token starts");
            }
            // Worker i receives on link i and sends on link i + 1.
            next.rotate_left(1);
            let (done_tx, done_rx) = async_channel::unbounded::<()>();
            for (rx, tx) in links.into_iter().zip(next) {
                let (hops, done) = (Arc::clone(&hops), done_tx.clone());
                outbox.tell(Effect::Spawn(Box::new(move |outbox| {
                    Box::pin(worker(outbox, rx, tx, hops, done))
                })));
            }
            drop(done_tx);
            while done_rx.recv().await.is_ok() {}
        })
    }
}

/// One machine of a ring. Each lap: take the token; race an ask against a
/// future that is already ready, so the ask is recorded and then abandoned
/// (a closed frame); ask again and check the reply is this request's own id;
/// pass the token on.
async fn worker(
    outbox: Outbox<Effect>,
    rx: Receiver<()>,
    tx: Sender<()>,
    hops: Arc<AtomicUsize>,
    _done: Sender<()>,
) {
    let mut id = 0;
    for _ in 0..LAPS {
        if rx.recv().await.is_err() {
            return;
        }
        select(outbox.ask(Effect::Ask), ready(())).await;
        id += 2;
        let answer = outbox.ask(Effect::Ask).await;
        assert_eq!(answer, id, "the reply to request {id}, and no other");
        hops.fetch_add(1, Ordering::SeqCst);
        // The last worker's last pass finds the next worker gone: fine.
        let _ = tx.try_send(());
    }
}

enum Event {
    Resume(u64),
    Reply(u64, u64),
}

/// The host's shared state.
#[derive(Default)]
struct Host {
    queue: VecDeque<Event>,
    in_flight: usize,
    live: HashSet<u64>,
    /// Requests reported closed, by machine and id.
    closed: BTreeSet<(u64, u64)>,
    /// Replies to closed requests the table refused: `STALE`, or `FINISHED`
    /// or `BAD_HANDLE` once the machine had completed or been freed. A reply
    /// to an open request is never refused: its routine awaits it.
    refused: usize,
    busy: usize,
    spurious: usize,
    outcome: Option<Result<(), String>>,
    dice: u64,
}

impl Host {
    /// xorshift64: the host's choices, from the seed. The remainder is less
    /// than `sides`, a `usize`, so it always converts.
    fn roll(&mut self, sides: usize) -> usize {
        self.dice ^= self.dice << 13;
        self.dice ^= self.dice >> 7;
        self.dice ^= self.dice << 17;
        usize::try_from(self.dice % sides as u64).unwrap_or_default()
    }

    /// Queue an event at the front or the back, as the dice say.
    fn push(&mut self, event: Event) {
        if self.roll(2) == 0 {
            self.queue.push_front(event);
        } else {
            self.queue.push_back(event);
        }
    }

    /// What one call's output asks of the host. Replies to closed requests
    /// are queued anyway: the table must refuse them as `STALE`.
    fn absorb(&mut self, handle: u64, bytes: &[u8]) -> Result<(), String> {
        let mut frames = Reader::new(bytes);
        while !frames.is_empty() {
            let kind = frames.u8().map_err(|e| e.to_string())?;
            let mut payload = Reader::new(frames.bytes().map_err(|e| e.to_string())?);
            match kind {
                FRAME_ASK => {
                    let (_, id) = (payload.u8(), payload.u64().map_err(|e| e.to_string())?);
                    self.push(Event::Reply(handle, id));
                }
                FRAME_TELL => {
                    let (_, child) = (payload.u8(), payload.u64().map_err(|e| e.to_string())?);
                    self.live.insert(child);
                    self.push(Event::Resume(child));
                }
                FRAME_CLOSED => {
                    let id = payload.u64().map_err(|e| e.to_string())?;
                    self.closed.insert((handle, id));
                }
                FRAME_WOKE => {
                    let woken = payload.u64().map_err(|e| e.to_string())?;
                    self.push(Event::Resume(woken));
                }
                other => return Err(format!("an unknown frame kind {other}")),
            }
        }
        Ok(())
    }

    /// Sometimes, resume a live machine for no reason: harmless, and it
    /// collides with other threads' calls.
    fn maybe_spurious(&mut self) {
        if self.spurious < 200 && !self.live.is_empty() && self.roll(6) == 0 {
            let pick = self.roll(self.live.len());
            if let Some(&handle) = self.live.iter().nth(pick) {
                self.spurious += 1;
                self.queue.push_back(Event::Resume(handle));
            }
        }
    }
}

/// A reply record for request `id`: kind 2 (`u64`), the id, the value.
fn u64_reply(id: u64, value: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(2);
    w.u64(id);
    w.u64(value);
    w.finish()
}

/// One host thread: take events until the run is decided.
fn serve(shared: &(Mutex<Host>, Condvar)) {
    let (lock, ready) = shared;
    loop {
        let event = {
            let mut host = lock.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if host.outcome.is_some() {
                    return;
                }
                if let Some(event) = host.queue.pop_front() {
                    host.in_flight += 1;
                    break event;
                }
                if host.in_flight == 0 {
                    // Quiet: whatever woke outside a call, or the end.
                    let wakes = table::wakes();
                    if let Err(e) = host.absorb(0, &wakes) {
                        host.outcome = Some(Err(e));
                    } else if host.queue.is_empty() {
                        host.outcome = Some(if host.live.is_empty() {
                            Ok(())
                        } else {
                            Err(format!("stalled with {} machines live", host.live.len()))
                        });
                    }
                    ready.notify_all();
                    continue;
                }
                host = ready.wait(host).unwrap_or_else(PoisonError::into_inner);
            }
        };

        // For a reply: its request, and whether it was closed.
        let (handle, result, reply) = match event {
            Event::Resume(handle) => (handle, table::resume(handle), None),
            Event::Reply(handle, id) => {
                let closed = lock
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .closed
                    .contains(&(handle, id));
                (
                    handle,
                    table::reply(handle, &u64_reply(id, id)),
                    Some((id, closed)),
                )
            }
        };

        let mut host = lock.lock().unwrap_or_else(PoisonError::into_inner);
        host.in_flight -= 1;
        let verdict = match (result, reply) {
            (Ok(_), Some((id, true))) => Err(format!(
                "a reply to closed request {id} of {handle} was accepted"
            )),
            (Ok((bytes, status)), _) => host.absorb(handle, &bytes).and_then(|()| {
                if status == Status::Complete && host.live.remove(&handle) {
                    table::free(handle).map_err(|e| format!("free {handle}: {e}"))
                } else {
                    Ok(())
                }
            }),
            (Err(Error::Stale { .. } | Error::Finished | Error::BadHandle), Some((_, true))) => {
                host.refused += 1;
                Ok(())
            }
            // Another thread is calling this machine: try again later.
            (Err(Error::Busy), Some((id, _))) => {
                host.busy += 1;
                host.queue.push_back(Event::Reply(handle, id));
                Ok(())
            }
            // A resume that collided, or found its machine finished or freed:
            // harmless, as resumes are.
            (Err(Error::Busy), None) => {
                host.busy += 1;
                host.queue.push_back(Event::Resume(handle));
                Ok(())
            }
            (Err(Error::Finished | Error::BadHandle), None) => Ok(()),
            (Err(e), _) => Err(format!("call on {handle}: {e}")),
        };
        if let Err(e) = verdict {
            host.outcome = Some(Err(e));
        }
        host.maybe_spurious();
        ready.notify_all();
    }
}

#[test]
fn a_multithreaded_host_runs_every_ring_to_completion() -> TestResult {
    for seed in 1..=SEEDS {
        let hops = Arc::new(AtomicUsize::new(0));
        let mut host = Host {
            dice: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            ..Host::default()
        };
        let threads = 2 + host.roll(3);
        for _ in 0..RINGS {
            let root = table::new(ring(Arc::clone(&hops)));
            host.live.insert(root);
            host.queue.push_back(Event::Resume(root));
        }

        let shared = Arc::new((Mutex::new(host), Condvar::new()));
        let pool: Vec<_> = (0..threads)
            .map(|_| {
                let shared = Arc::clone(&shared);
                thread::spawn(move || serve(&shared))
            })
            .collect();
        for thread in pool {
            thread.join().map_err(|_| "a host thread panicked")?;
        }

        let host = shared.0.lock()?;
        if let Some(Err(e)) = &host.outcome {
            Err(format!("seed {seed}, {threads} threads: {e}"))?;
        }
        assert_eq!(
            hops.load(Ordering::SeqCst),
            RINGS * WORKERS * LAPS,
            "seed {seed}"
        );
        assert_eq!(
            host.refused,
            RINGS * WORKERS * LAPS,
            "every reply to an abandoned ask was refused"
        );
        assert!(table::wakes().is_empty(), "nothing woke after the end");
    }
    Ok(())
}
