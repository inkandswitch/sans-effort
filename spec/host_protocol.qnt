// -*- mode: Bluespec; -*-

// The host protocol of `ABI.md`, as a state machine: a library that runs
// machines one call at a time, and a host that drives them, possibly with
// several calls in flight on different workers.
//
// Routines are abstracted to programs over a few instructions. What is
// modelled exactly is what the ABI promises about calls: a wake during a
// call is a `woke` frame in that call's output; a wake outside any call — or
// during one that fails — waits for `wakes`; a race's loser closes; a child
// exists only if the call that spawned it returned; a pinned machine stays
// on the worker that began it.
//
// The host is the recommended shape: at most one call per machine, a pinned
// machine's calls on its own worker, outputs handled as calls return, closed
// frames dropping replies not yet sent, and when nothing is left, `wakes`,
// then a verdict, "end" or "stall". It may also resume a waiting machine for
// no reason, or free one, within a budget: the ABI says both are safe.
//
// Check with `nix develop --command spec:check`.

module types {
  // One step of a routine.
  type Instr =
    // Send a message on a channel. Channels are unbounded: never blocks.
    | Send(str)
    // Take a message. A closed, empty channel ends the routine.
    | Recv(str)
    // Start a child running a program: a tell, which the host begins.
    | Spawn(str)
    // The same, pinned: the child stays on the worker that begins it.
    | SpawnPinned(str)
    // Ask the host and wait for the reply.
    | Ask
    // Receive on a channel and ask the host together, as
    // `select(recv, ask)`: whichever is ready first wins, the receive on a
    // tie; the loser is dropped — a losing ask closes.
    | Race(str)
    // Panic. The machine is removed; what it held is dropped.
    | Panic
}

module protocol {
  import types.*

  // The programs, by name.
  const PROGRAMS: str -> List[Instr]
  // The channels whose senders each program holds, from its start to its end.
  const HOLDS: str -> Set[str]
  const CHANNELS: Set[str]
  // The program the host creates with `new`.
  const ROOT: str
  // Calls the host may have in flight at once; 1 is a sequential host.
  const WORKERS: int
  // The verdict the host must reach: "end" or "stall".
  const EXPECTED: str
  // The host resumes what `woke` frames name. (False: a host that ignores them.)
  const HEEDS_WOKE: bool
  // A closed frame drops the reply the host had queued. (False: a host that
  // replies to a request it was told had closed.)
  const DROPS_CLOSED: bool
  // A pinned machine's calls go to the worker that began it. (False: a host
  // that calls on any worker, which the table refuses as `WRONG_THREAD`.)
  const ROUTES_PINNED: bool
  // The end is declared only once every machine is done. (False: a host that
  // calls it the end whenever it has nothing left to do.)
  const CHECKS_ALL_DONE: bool
  // An idle machine is resumed when a `woke` frame or `wakes` names it.
  // (False: a host that resumes a machine whenever its call returns idle.)
  const WAITS_FOR_WOKE: bool
  // Resumes the host may make for no reason, as a seeded host does.
  const SPURIOUS: int
  // Waiting machines the host may free before they finish, as the testing
  // runner's `Schedule::kill` does.
  const KILLS: int

  // Enough instructions to run any one call to its next wait.
  pure val STEPS = 8
  // More host actions than any instance's longest run takes.
  pure val BOUND = 60

  // Count a host action, stopping one past BOUND: a run that goes on
  // forever then repeats its states, a cycle the liveness check can find,
  // and has still broken `terminates`.
  pure def tick(steps: int): int = if (steps > BOUND) steps else steps + 1

  type Status =
    | NotStarted
    | Running
    | Awaiting
    | Idle
    | Panicking
    | Complete
    | Gone

  type Machine = {
    prog: str,
    pc: int,
    status: Status,
    // The channel it waits on, or "".
    waiting: str,
    // Woken since it began to wait: once per wait.
    woken: bool,
    // Requests outstanding, by id.
    asks: Set[int],
    // The ask of a race still undecided, or 0.
    racing: int,
    next_id: int,
    holds: Set[str],
    // Stays on the worker that begins it: `owner`, or -1 before that.
    pinned: bool,
    owner: int,
  }

  // An effect a call records: a child spawned, or a request asked.
  type Effect =
    // The child's handle once the call returns; its index in the call's
    // `children` until then.
    | Spawned(int)
    // The request's id.
    | Asked(int)

  // The library while a call runs, and what the call has emitted so far.
  type World = {
    machines: int -> Machine,
    queued: str -> int,
    senders: str -> int,
    woke: Set[int],
    // In the order they were recorded.
    effects: List[Effect],
    closed: Set[int],
    // Children spawned in this call, registered when it returns: each
    // program, and whether it is pinned.
    children: List[(str, bool)],
  }

  type Event =
    | Resume(int)
    | Reply((int, int))

  // A call's output, delivered to the host when the call returns.
  type Output = {
    woke: Set[int],
    effects: List[Effect],
    closed: Set[int],
    status: Status,
    // The worker the call runs on.
    worker: int,
  }

  // ---- The library ------------------------------------------------------

  var machines: int -> Machine
  var queued: str -> int
  var senders: str -> int
  // Woken outside any call, or during a failed one: for `wakes`.
  var wakes: Set[int]

  // ---- The host ---------------------------------------------------------

  // What the host will do: resume a machine, or send a reply it has ready.
  var pending: Set[Event]
  // Calls in progress, by machine, with the output they will return.
  var in_flight: int -> Output
  var verdict: str
  // Replies the host sent to a request already closed (`STALE`).
  var stale: int
  // Calls the table refused as on the wrong worker (`WRONG_THREAD`).
  var wrong_thread: int
  // Host actions taken so far: bounded exactly when every run terminates.
  var steps: int
  // What is left of the budgets for spurious resumes and kills.
  var spurious_left: int
  var kills_left: int

  pure def fresh(prog: str, pinned: bool): Machine = {
    prog: prog,
    pc: 0,
    status: NotStarted,
    waiting: "",
    woken: false,
    asks: Set(),
    racing: 0,
    next_id: 1,
    holds: HOLDS.get(prog),
    pinned: pinned,
    owner: -1,
  }

  pure def update(w: World, h: int, f: Machine => Machine): World =
    { ...w, machines: w.machines.set(h, f(w.machines.get(h))) }

  // Wake every machine waiting on `c` that is not already woken; the call
  // running now reports each in its output.
  pure def wake_waiters(w: World, c: str): World =
    w.machines.keys().fold(w, (acc, h) => {
      val m = acc.machines.get(h)
      if (m.waiting == c and not(m.woken))
        { ...update(acc, h, mm => { ...mm, woken: true }), woke: acc.woke.union(Set(h)) }
      else acc
    })

  // Drop a sender of each of `chans`; a channel whose last sender goes
  // closes, which wakes its waiters.
  pure def drop_senders(w: World, chans: Set[str]): World =
    chans.fold(w, (acc, c) => {
      val left = acc.senders.get(c) - 1
      val dropped = { ...acc, senders: acc.senders.set(c, left) }
      if (left == 0) wake_waiters(dropped, c) else dropped
    })

  // A channel a receive would not wait on: a message, or closed.
  pure def ready(w: World, c: str): bool =
    w.queued.get(c) > 0 or w.senders.get(c) == 0

  // Take a message if there is one (a closed, empty channel has none).
  pure def take(w: World, c: str): World =
    if (w.queued.get(c) > 0) { ...w, queued: w.queued.set(c, w.queued.get(c) - 1) } else w

  // The machine returns: its outstanding asks close, what it held drops.
  pure def finish(w: World, h: int): World = {
    val m = w.machines.get(h)
    val done = update(w, h, mm => { ...mm, status: Complete, asks: Set(), racing: 0, waiting: "", holds: Set() })
    drop_senders({ ...done, closed: done.closed.union(m.asks) }, m.holds)
  }

  // The child's closure holds its senders already; the table registers it
  // only when this call returns.
  pure def spawn(w: World, h: int, p: str, pinned: bool): World =
    { ...update(w, h, mm => { ...mm, pc: mm.pc + 1 }),
      senders: HOLDS.get(p).fold(w.senders, (s, c) => s.set(c, s.get(c) + 1)),
      effects: w.effects.append(Spawned(w.children.length())),
      children: w.children.append((p, pinned)) }

  // Run one instruction of a running machine.
  pure def exec(w: World, h: int): World = {
    val m = w.machines.get(h)
    val program = PROGRAMS.get(m.prog)
    if (m.status != Running) w
    else if (m.pc >= program.length()) finish(w, h)
    else match program[m.pc] {
      | Send(c) =>
        wake_waiters(update({ ...w, queued: w.queued.set(c, w.queued.get(c) + 1) }, h, mm => { ...mm, pc: mm.pc + 1 }), c)
      | Recv(c) =>
        if (w.queued.get(c) > 0) update(take(w, c), h, mm => { ...mm, pc: mm.pc + 1 })
        else if (w.senders.get(c) == 0) finish(w, h)
        else update(w, h, mm => { ...mm, status: Idle, waiting: c, woken: false })
      | Spawn(p) => spawn(w, h, p, false)
      | SpawnPinned(p) => spawn(w, h, p, true)
      | Ask =>
        { ...update(w, h, mm => { ...mm, status: Awaiting, asks: mm.asks.union(Set(mm.next_id)), next_id: mm.next_id + 1 }),
          effects: w.effects.append(Asked(m.next_id)) }
      | Race(c) =>
        if (ready(w, c)) update(take(w, c), h, mm => { ...mm, pc: mm.pc + 1 })
        else
          { ...update(w, h, mm => { ...mm, status: Awaiting, waiting: c, woken: false, racing: mm.next_id,
                                   asks: mm.asks.union(Set(mm.next_id)), next_id: mm.next_id + 1 }),
            effects: w.effects.append(Asked(m.next_id)) }
      | Panic =>
        update(w, h, mm => { ...mm, status: Panicking })
    }
  }

  // Run a machine to its next wait.
  pure def run_to_wait(w: World, h: int): World = range(0, STEPS).foldl(w, (acc, _) => exec(acc, h))

  // A resume: begin a machine, or let a waiting one look again. Harmless
  // when nothing changed.
  pure def resume(w: World, h: int): World = {
    val m = w.machines.get(h)
    match m.status {
      | NotStarted => update(w, h, mm => { ...mm, status: Running })
      | Idle => update(w, h, mm => { ...mm, status: Running, waiting: "", woken: false })
      | Awaiting =>
        if (m.racing != 0 and w.queued.get(m.waiting) > 0)
          // The receive won the race: the ask closes.
          { ...update(take(w, m.waiting), h, mm => { ...mm, status: Running, pc: mm.pc + 1, waiting: "", woken: false,
                                                     asks: mm.asks.exclude(Set(mm.racing)), racing: 0 }),
            closed: w.closed.union(Set(m.racing)) }
        else if (m.racing != 0) update(w, h, mm => { ...mm, woken: false })
        else w
      | _ => w
    }
  }

  // A reply: delivered if the request is outstanding.
  pure def deliver(w: World, h: int, id: int): World = {
    val m = w.machines.get(h)
    if (not(m.asks.contains(id))) w
    else if (id == m.racing and w.queued.get(m.waiting) > 0)
      // Both ready: `select` polls the receive first, and it wins; the
      // answered ask has nothing left to close.
      update(take(w, m.waiting), h, mm => { ...mm, status: Running, pc: mm.pc + 1, waiting: "", woken: false,
                                           asks: mm.asks.exclude(Set(id)), racing: 0 })
    else
      update(w, h, mm => { ...mm, status: Running, pc: mm.pc + 1, waiting: "", woken: false,
                          asks: mm.asks.exclude(Set(id)), racing: 0 })
  }

  // After a panic: the machine is removed, outstanding asks close without
  // frames, what it held drops; every wake of the failed call waits for
  // `wakes`, and the children it spawned never existed.
  pure def fail_call(w: World, h: int): World = {
    val m = w.machines.get(h)
    val unspawned = w.children.foldl(Set(), (s, c) => s.union(HOLDS.get(c._1)))
    val gone = update(w, h, mm => { ...mm, status: Gone, asks: Set(), racing: 0, waiting: "", holds: Set() })
    drop_senders(drop_senders(gone, m.holds), unspawned)
  }

  // Register the children a successful call spawned, numbered in order,
  // and name each in its effect by its handle.
  pure def register(w: World): World = {
    val base = w.machines.keys().size()
    { ...w,
      machines: w.children.foldl(w.machines, (ms, c) => ms.put(ms.keys().size(), fresh(c._1, c._2))),
      effects: w.effects.foldl([], (es, e) => es.append(match e {
        | Spawned(i) => Spawned(base + i)
        | Asked(id) => Asked(id)
      })) }
  }

  // The children an output spawned, and the requests it asked.
  pure def spawned_in(effects: List[Effect]): Set[int] =
    effects.foldl(Set(), (s, e) => match e { | Spawned(c) => s.union(Set(c)) | Asked(_) => s })
  pure def asked_in(effects: List[Effect]): Set[int] =
    effects.foldl(Set(), (s, e) => match e { | Spawned(_) => s | Asked(id) => s.union(Set(id)) })

  // A machine waiting: the host has seen it, and may resume or free it.
  pure def waiting_in(m: Machine): bool = m.status == Idle or m.status == Awaiting

  pure def target(e: Event): int = match e { | Resume(h) => h | Reply(r) => r._1 }

  // Workers with no call in flight.
  def idle_workers: Set[int] =
    0.to(WORKERS - 1).exclude(in_flight.keys().map(k => in_flight.get(k).worker))

  // ---- The host's actions ------------------------------------------------

  action init = all {
    machines' = Map(0 -> fresh(ROOT, false)),
    queued' = CHANNELS.mapBy(_ => 0),
    senders' = CHANNELS.mapBy(c => if (HOLDS.get(ROOT).contains(c)) 1 else 0),
    wakes' = Set(),
    pending' = Set(Resume(0)),
    in_flight' = Map(),
    verdict' = "running",
    stale' = 0,
    wrong_thread' = 0,
    steps' = 0,
    spurious_left' = SPURIOUS,
    kills_left' = KILLS,
  }

  // Make a call: on a worker that is free, for a machine with no call in
  // flight. The call runs at once; its output reaches the host on `ret`. A
  // pinned machine is begun on the worker of its first call, and refuses any
  // other after (`WRONG_THREAD`): nothing changes, and a host that makes such
  // a call has lost the event.
  action call = {
    nondet e = pending.oneOf()
    nondet worker = idle_workers.oneOf()
    val h = target(e)
    val m = machines.get(h)
    val wrong = m.pinned and m.owner != -1 and m.owner != worker
    val w0: World = { machines: machines, queued: queued, senders: senders,
                      woke: Set(), effects: [], closed: Set(), children: [] }
    val w1 = match e { | Resume(_) => resume(w0, h) | Reply(r) => deliver(w0, h, r._2) }
    val w2 = run_to_wait(w1, h)
    val failed = w2.machines.get(h).status == Panicking
    val w3 = if (failed) fail_call(w2, h) else register(w2)
    val owned = if (m.pinned and m.owner == -1) update(w3, h, mm => { ...mm, owner: worker }) else w3
    val is_stale = match e { | Reply(r) => not(m.asks.contains(r._2)) | _ => false }
    all {
      verdict == "running",
      not(in_flight.keys().contains(h)),
      in_flight.keys().size() < WORKERS,
      ROUTES_PINNED implies not(wrong),
      pending' = pending.exclude(Set(e)),
      verdict' = verdict,
      steps' = tick(steps),
      spurious_left' = spurious_left,
      kills_left' = kills_left,
      if (wrong) all {
        machines' = machines,
        queued' = queued,
        senders' = senders,
        wakes' = wakes,
        in_flight' = in_flight,
        stale' = stale,
        wrong_thread' = wrong_thread + 1,
      } else all {
        machines' = owned.machines,
        queued' = owned.queued,
        senders' = owned.senders,
        wakes' = if (failed) wakes.union(owned.woke) else wakes,
        in_flight' = in_flight.put(h, {
          woke: if (failed) Set() else owned.woke,
          effects: if (failed) [] else owned.effects,
          closed: if (failed) Set() else owned.closed,
          status: owned.machines.get(h).status,
          worker: worker,
        }),
        stale' = if (is_stale) stale + 1 else stale,
        wrong_thread' = wrong_thread,
      },
    }
  }

  // A call returns, and the host acts on its output: resume what woke,
  // begin each child, queue a reply for each ask, drop replies to what
  // closed, and free the machine if it is done.
  action ret = {
    nondet h = in_flight.keys().oneOf()
    val out = in_flight.get(h)
    val woken = if (HEEDS_WOKE) out.woke.map(m => Resume(m)) else Set()
    val polled = if (not(WAITS_FOR_WOKE) and out.status == Idle) Set(Resume(h)) else Set()
    val dropped = if (DROPS_CLOSED) out.closed.map(id => Reply((h, id))) else Set()
    val ended = out.status == Complete or out.status == Gone
    all {
      in_flight' = in_flight.keys().exclude(Set(h)).mapBy(k => in_flight.get(k)),
      pending' = pending
        .union(woken)
        .union(polled)
        .union(spawned_in(out.effects).map(c => Resume(c)))
        .union(asked_in(out.effects).map(id => Reply((h, id))))
        .exclude(dropped),
      machines' = if (ended) machines.set(h, { ...machines.get(h), status: Gone }) else machines,
      queued' = queued,
      senders' = senders,
      wakes' = wakes,
      verdict' = verdict,
      stale' = stale,
      wrong_thread' = wrong_thread,
      steps' = tick(steps),
      spurious_left' = spurious_left,
      kills_left' = kills_left,
    }
  }

  // Nothing to call, nothing in flight: ask `wakes`. If it names nobody,
  // nothing can happen again — the end, or a stall.
  action quiet = all {
    verdict == "running",
    pending == Set(),
    in_flight.keys() == Set(),
    if (wakes != Set()) all { pending' = wakes.map(h => Resume(h)), wakes' = Set(), verdict' = verdict }
    else all {
      pending' = pending,
      wakes' = wakes,
      verdict' = if (not(CHECKS_ALL_DONE) or machines.keys().forall(h => machines.get(h).status == Gone)) "end"
                 else "stall",
    },
    machines' = machines,
    queued' = queued,
    senders' = senders,
    in_flight' = in_flight,
    stale' = stale,
    wrong_thread' = wrong_thread,
    steps' = tick(steps),
    spurious_left' = spurious_left,
    kills_left' = kills_left,
  }

  // A resume for no reason, of a waiting machine with no call in flight. The
  // ABI says it is harmless; the properties holding with it prove so.
  action spurious = {
    nondet h = machines.keys().filter(h => waiting_in(machines.get(h)) and not(in_flight.keys().contains(h))).oneOf()
    all {
      verdict == "running",
      spurious_left > 0,
      pending' = pending.union(Set(Resume(h))),
      spurious_left' = spurious_left - 1,
      machines' = machines,
      queued' = queued,
      senders' = senders,
      wakes' = wakes,
      in_flight' = in_flight,
      verdict' = verdict,
      stale' = stale,
      wrong_thread' = wrong_thread,
      steps' = tick(steps),
      kills_left' = kills_left,
    }
  }

  // The host frees a waiting machine with no call in flight. As after a
  // panic, but outside any call: its requests close without frames, what it
  // held drops, and the wakes that causes wait for `wakes`. The host forgets
  // the events it had for the machine.
  action kill = {
    nondet h = machines.keys().filter(h => waiting_in(machines.get(h)) and not(in_flight.keys().contains(h))).oneOf()
    val m = machines.get(h)
    val w0: World = { machines: machines, queued: queued, senders: senders,
                      woke: Set(), effects: [], closed: Set(), children: [] }
    val gone = update(w0, h, mm => { ...mm, status: Gone, asks: Set(), racing: 0, waiting: "", holds: Set() })
    val w1 = drop_senders(gone, m.holds)
    all {
      verdict == "running",
      kills_left > 0,
      machines' = w1.machines,
      queued' = w1.queued,
      senders' = w1.senders,
      wakes' = wakes.union(w1.woke),
      pending' = pending.filter(e => target(e) != h),
      kills_left' = kills_left - 1,
      in_flight' = in_flight,
      verdict' = verdict,
      stale' = stale,
      wrong_thread' = wrong_thread,
      steps' = tick(steps),
      spurious_left' = spurious_left,
    }
  }

  // The host has given its verdict; nothing more happens.
  action done = all {
    verdict != "running",
    machines' = machines,
    queued' = queued,
    senders' = senders,
    wakes' = wakes,
    pending' = pending,
    in_flight' = in_flight,
    verdict' = verdict,
    stale' = stale,
    wrong_thread' = wrong_thread,
    steps' = steps,
    spurious_left' = spurious_left,
    kills_left' = kills_left,
  }

  action step = any { call, ret, quiet, spurious, kill, done }

  // ---- Properties --------------------------------------------------------

  // The library would let this machine progress if it were called.
  def runnable(h: int): bool = {
    val m = machines.get(h)
    val world: World = { machines: machines, queued: queued, senders: senders,
                         woke: Set(), effects: [], closed: Set(), children: [] }
    match m.status {
      | NotStarted => true
      | Idle => ready(world, m.waiting)
      | Awaiting => m.racing != 0 and ready(world, m.waiting)
      | _ => false
    }
  }

  // No wake is lost: a machine that could progress is about to be called,
  // or will be named by `wakes` or by a call still in flight.
  val no_lost_wake = machines.keys().forall(h => runnable(h) implies (
    pending.contains(Resume(h))
      or wakes.contains(h)
      or in_flight.keys().contains(h)
      or in_flight.keys().exists(x => in_flight.get(x).woke.contains(h) or spawned_in(in_flight.get(x).effects).contains(h))
  ))

  // A stall is declared only when nothing could ever progress.
  val stall_is_sound = verdict == "stall" implies machines.keys().forall(h => not(runnable(h)))

  // The end is declared only when every machine is done.
  val end_is_sound = verdict == "end" implies machines.keys().forall(h => machines.get(h).status == Gone)

  // Between calls no machine is mid-run: each call ran its machine to a wait
  // (`STEPS` is enough for every program), and a panic removed it.
  val settled = machines.keys().forall(h => machines.get(h).status != Running and machines.get(h).status != Panicking)

  // The host never replies to a request it was told had closed.
  val no_stale_reply = stale == 0

  // The host never calls a pinned machine on another worker than its own.
  val no_wrong_thread = wrong_thread == 0

  val safe = no_lost_wake and stall_is_sound and end_is_sound and no_stale_reply and no_wrong_thread and settled

  // Liveness, as safety over the whole state graph, checkable exhaustively
  // and needing no fairness: no run is longer than BOUND (a run that could
  // go on forever would exceed it), and the verdict, when it comes, is the
  // expected one. That the host can always act until its verdict follows from
  // the actions' guards (nothing in flight leaves every worker free; nothing
  // pending enables `quiet`; a call in flight enables `ret`), and TLC's own
  // deadlock check, in the liveness run below, would catch a state where it
  // cannot; `done` keeps the verdict's states enabled. So every run reaches
  // the expected verdict.
  val terminates = steps <= BOUND
  val right_verdict = verdict == "running" or verdict == EXPECTED

  val ends_right = terminates and right_verdict

  // The same, as a temporal property under weak fairness: the host always
  // reaches the verdict it should. `spec:check` runs it on TLC directly, not
  // through `quint verify`, where it never finishes: Apalache's folds, which
  // `run_to_wait` compiles to, are made strict first (see `strict-folds` in
  // nix/commands.nix).
  temporal fair = weakFair(call, (machines, queued, senders, wakes, pending, in_flight, verdict, stale, wrong_thread, steps, spurious_left, kills_left))
    and weakFair(ret, (machines, queued, senders, wakes, pending, in_flight, verdict, stale, wrong_thread, steps, spurious_left, kills_left))
    and weakFair(quiet, (machines, queued, senders, wakes, pending, in_flight, verdict, stale, wrong_thread, steps, spurious_left, kills_left))

  temporal reaches_verdict = fair implies eventually(verdict == EXPECTED)
}

// ---- Instances --------------------------------------------------------------

// Two rounds of ping-pong: each round's pong is a send no effect reports;
// only a `woke` frame tells the host to resume the parent. The host may
// resume a machine twice for no reason, and free one before it finishes:
// freeing either closes the other's channel, so the run still ends.
module ping_pong {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("ponger"), Send("ping"), Recv("pong"), Send("ping"), Recv("pong")],
      "ponger" -> [Recv("ping"), Send("pong"), Recv("ping"), Send("pong"), Recv("ping")],
    ),
    HOLDS = Map("parent" -> Set("ping"), "ponger" -> Set("pong")),
    CHANNELS = Set("ping", "pong"),
    ROOT = "parent",
    WORKERS = 2,
    EXPECTED = "end",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 2,
    KILLS = 1,
  ).*
}

module ping_pong_sequential {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("ponger"), Send("ping"), Recv("pong"), Send("ping"), Recv("pong")],
      "ponger" -> [Recv("ping"), Send("pong"), Recv("ping"), Send("pong"), Recv("ping")],
    ),
    HOLDS = Map("parent" -> Set("ping"), "ponger" -> Set("pong")),
    CHANNELS = Set("ping", "pong"),
    ROOT = "parent",
    WORKERS = 1,
    EXPECTED = "end",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 1,
    KILLS = 1,
  ).*
}

// A race: a receive from a quick worker against an ask (a sleep). Either
// may win; a losing ask closes. Then one more ask.
module deadline {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("quick"), Race("answer"), Ask],
      "quick" -> [Send("answer")],
    ),
    HOLDS = Map("parent" -> Set(), "quick" -> Set("answer")),
    CHANNELS = Set("answer"),
    ROOT = "parent",
    WORKERS = 2,
    EXPECTED = "end",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 1,
    KILLS = 1,
  ).*
}

// A child panics holding the sender its parent waits on: the failed call
// has no output, so only `wakes` can tell the host to resume the parent.
module panic_wakes {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("crasher"), Recv("signal")],
      "crasher" -> [Panic],
    ),
    HOLDS = Map("parent" -> Set(), "crasher" -> Set("signal")),
    CHANNELS = Set("signal"),
    ROOT = "parent",
    WORKERS = 2,
    EXPECTED = "end",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 1,
    KILLS = 1,
  ).*
}

// Each waits for the other, holding the sender the other waits on: the host
// must declare a stall, not hang and not call it the end.
module deadlock {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("other"), Recv("from_other")],
      "other" -> [Recv("from_parent")],
    ),
    HOLDS = Map("parent" -> Set("from_parent"), "other" -> Set("from_other")),
    CHANNELS = Set("from_parent", "from_other"),
    ROOT = "parent",
    WORKERS = 2,
    EXPECTED = "stall",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 1,
    KILLS = 0,
  ).*
}

// A ring of three, the token twice round: each node takes it from the one
// before and passes it on. When the root ends, its sender drops, and the
// ring closes one node at a time.
module ring {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "node0" -> [Spawn("node1"), Spawn("node2"), Send("c1"), Recv("c0"), Send("c1"), Recv("c0")],
      "node1" -> [Recv("c1"), Send("c2"), Recv("c1"), Send("c2"), Recv("c1")],
      "node2" -> [Recv("c2"), Send("c0"), Recv("c2"), Send("c0"), Recv("c2")],
    ),
    HOLDS = Map("node0" -> Set("c1"), "node1" -> Set("c2"), "node2" -> Set("c0")),
    CHANNELS = Set("c0", "c1", "c2"),
    ROOT = "node0",
    WORKERS = 2,
    EXPECTED = "end",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 1,
    KILLS = 1,
  ).*
}

// The front desk with two visitors: it reads each name (an ask), spawns a
// pinned clerk for it, reads the end of input, then waits for each clerk's
// greeting in order. A clerk looks its name up (an ask) and sends the
// greeting. Each clerk stays on the worker that began it. The lines written
// are tells, which the protocol does not see.
module front_desk {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "desk" -> [Ask, SpawnPinned("clerk_a"), Ask, SpawnPinned("clerk_b"), Ask, Recv("greet_a"), Recv("greet_b")],
      "clerk_a" -> [Ask, Send("greet_a")],
      "clerk_b" -> [Ask, Send("greet_b")],
    ),
    HOLDS = Map("desk" -> Set(), "clerk_a" -> Set("greet_a"), "clerk_b" -> Set("greet_b")),
    CHANNELS = Set("greet_a", "greet_b"),
    ROOT = "desk",
    WORKERS = 2,
    EXPECTED = "end",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 1,
    KILLS = 1,
  ).*
}

// ---- Hosts that get it wrong: these must fail ------------------------------

// A host that ignores `woke` frames loses the ponger's silent send.
module mistaken_ignores_woke {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("ponger"), Send("ping"), Recv("pong"), Send("ping"), Recv("pong")],
      "ponger" -> [Recv("ping"), Send("pong"), Recv("ping"), Send("pong"), Recv("ping")],
    ),
    HOLDS = Map("parent" -> Set("ping"), "ponger" -> Set("pong")),
    CHANNELS = Set("ping", "pong"),
    ROOT = "parent",
    WORKERS = 2,
    EXPECTED = "end",
    HEEDS_WOKE = false,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 0,
    KILLS = 0,
  ).*
}

// A host that keeps a reply queued after its request closed sends it anyway.
module mistaken_ignores_closed {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("quick"), Race("answer"), Ask],
      "quick" -> [Send("answer")],
    ),
    HOLDS = Map("parent" -> Set(), "quick" -> Set("answer")),
    CHANNELS = Set("answer"),
    ROOT = "parent",
    WORKERS = 2,
    EXPECTED = "end",
    HEEDS_WOKE = true,
    DROPS_CLOSED = false,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 0,
    KILLS = 0,
  ).*
}

// A host that calls a pinned clerk on whichever worker is free: the table
// refuses the call, and the host has lost the event.
module mistaken_ignores_pinning {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "desk" -> [Ask, SpawnPinned("clerk_a"), Ask, SpawnPinned("clerk_b"), Ask, Recv("greet_a"), Recv("greet_b")],
      "clerk_a" -> [Ask, Send("greet_a")],
      "clerk_b" -> [Ask, Send("greet_b")],
    ),
    HOLDS = Map("desk" -> Set(), "clerk_a" -> Set("greet_a"), "clerk_b" -> Set("greet_b")),
    CHANNELS = Set("greet_a", "greet_b"),
    ROOT = "desk",
    WORKERS = 2,
    EXPECTED = "end",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = false,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 0,
    KILLS = 0,
  ).*
}

// A host that calls it the end whenever it has nothing left to do mistakes a
// deadlock for the end.
module mistaken_ends_when_idle {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("other"), Recv("from_other")],
      "other" -> [Recv("from_parent")],
    ),
    HOLDS = Map("parent" -> Set("from_parent"), "other" -> Set("from_other")),
    CHANNELS = Set("from_parent", "from_other"),
    ROOT = "parent",
    WORKERS = 2,
    EXPECTED = "stall",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = false,
    WAITS_FOR_WOKE = true,
    SPURIOUS = 0,
    KILLS = 0,
  ).*
}

// A host that resumes a machine whenever its call returns idle, rather than
// waiting to be told it may progress, spins forever on a deadlock and never
// reaches a verdict.
module mistaken_polls_idle {
  import types.*
  import protocol(
    PROGRAMS = Map(
      "parent" -> [Spawn("other"), Recv("from_other")],
      "other" -> [Recv("from_parent")],
    ),
    HOLDS = Map("parent" -> Set("from_parent"), "other" -> Set("from_other")),
    CHANNELS = Set("from_parent", "from_other"),
    ROOT = "parent",
    WORKERS = 2,
    EXPECTED = "stall",
    HEEDS_WOKE = true,
    DROPS_CLOSED = true,
    ROUTES_PINNED = true,
    CHECKS_ALL_DONE = true,
    WAITS_FOR_WOKE = false,
    SPURIOUS = 0,
    KILLS = 0,
  ).*
}
