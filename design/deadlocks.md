# Deadlocks

A deadlock is a set of machines, or threads, each waiting on another in the set, so none can move. There are four ways to build one with `sans-effort`, and they need different tools. One fact shapes all of them: channels between machines are plain Rust channels, which the library does not see into, so it cannot prevent every deadlock. What it guarantees instead is that a driven host _notices_ one and reports it rather than hanging.

| Kind                                              | Example                                                 | Caught by                       | Prevented by                        |
|---------------------------------------------------|---------------------------------------------------------|---------------------------------|-------------------------------------|
| [Waits on channels](#waits-on-channels)           | Two machines, each waiting to hear from the other first | The host's stall verdict        | Nothing in the library (see below)  |
| [Locks across a wait](#locks-across-a-wait)       | A routine keeps a lock while it waits on a reply        | The compiler, for most routines | A rule: never wait holding a lock   |
| [Locks between machines](#locks-between-machines) | Two machines take locks A and B in opposite orders      | Nothing                         | A lock-ordering library: `surelock` |
| [The host's own locks](#the-hosts-own-locks)      | (none)                                                  | (none needed)                   | The handle table's design           |

## Waits on Channels

Two machines each wait on a channel that only the other can send on, and neither sends first. `routines::faults::Deadlock` in `demo/` does this on purpose. Neither machine is blocked in the operating-system sense: each has simply returned `IDLE`, and nothing will ever wake it.

_Caught._ A driven host knows when nothing more can happen: no call to make, no request outstanding, and nothing from `wakes`. At that point either every machine has completed, or the rest can never progress: a stall. A host should report the stall rather than wait (`ABI.md` §Writing a Host), and the model checker proves the recommended loop gives that verdict only when it is true. `testing::drive` returns `Err(Stalled)` naming the stuck machines, and each of the demo's driven hosts reports the deadlock of `--deadlock` and exits. Killing either machine breaks this deadlock: the other hears its channel close and finishes. A native host has no such detector: under tokio the same routines simply hang.

_Not prevented._ A lock ladder (each machine at a level; waits only go one way) would reject correct protocols (in ping-pong the child waits on its parent, and the parent waits on its child, in turn), and it cannot see a send that never happens. The sound tools, session types or priority-and-obligation type systems, would restrict what plain channels can do. So the stall verdict is the safety net, and it is a check every host makes.

## Locks Across a Wait

A routine takes a lock and then awaits a reply, a message, or a timer while still holding it. Under a driver, an await on an effect means the routine suspends until the host answers, possibly much later, possibly on another thread. Anything else that wants the lock meanwhile waits for as long as the host does; if the host's answer depends on that other party, nothing moves.

_The rule:_ never suspend a routine while it holds a lock. Take the lock, use it, and release it within one poll: between two awaits, not across one.

_Caught by the compiler, mostly._ A migrating machine's future must be `Send`, and `std::sync::MutexGuard` is not, so a `std` guard held across an `.await` does not compile in one. Two cases slip through: a pinned machine (whose future need not be `Send`), and async mutexes, whose guards are `Send` by design. For those, the rule is the only defence.

## Locks Between Machines

On a parallel host, two machines run at once on different threads. If one takes lock A and then lock B within a poll, and the other takes B and then A, each can end up holding the lock the other needs. This is the classic deadlock, and nothing about machines or effects changes it.

_Prevented by ordering._ [`surelock`](https://crates.io/crates/surelock) makes it impossible by construction, and needs nothing from `sans-effort`: a routine uses its `Mutex` in its own state. It offers two tools:

- _Levels:_ each mutex has a level in its type, and a key that tracks the current level is consumed and re-issued by each acquisition, so taking locks out of order is a compile error.
- _`LockSet`:_ several locks at the same level are acquired together, always in one stable order.

Its lock calls are synchronous, so the rule above still applies: a `surelock` guard, like any guard, is released before the routine awaits.

## The Host's Own Locks

`sans-effort-host`'s handle table is shared by every thread a host uses. Its locks cannot deadlock each other, and a routine may call into it:

- _A machine's own lock_ is only ever tried, never waited on. Two threads calling one handle at once get `BUSY`, so this lock never takes part in a wait cycle. A routine that calls the table on its own handle gets `BUSY` too.
- _The table's lock_ is held briefly: to find a machine, to register a new one, or to remove one. A routine runs with it released, so a routine (or a binding's `split`, mid-poll) may spawn, which registers a child. Removing a machine releases the lock before dropping the machine, so a routine's destructors may call the table too.
- _The list of wakes reported by `wakes`_ is the innermost lock: nothing else is taken while it is held.

So the order is fixed (a machine's lock, then the table's, then the wakes), and the first is never waited on.
