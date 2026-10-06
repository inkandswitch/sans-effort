// -*- mode: Bluespec; -*-

// The host protocol of `ABI.md`, as a state machine: a library that runs
// machines one call at a time, and a host that drives them, possibly with
// several calls in flight on different workers.
//
// Routines are abstracted to programs over a few instructions. What is
// modelled exactly is what the ABI promises about calls: a wake during a
// call is a `woke` frame in that call's output; a wake outside any call — or
// during one that fails — waits for `wakes`; a race's loser closes; a child
// exists only if the call that spawned it returned.
//
// The host is the recommended shape: at most one call per machine, outputs
// handled as calls return, closed frames dropping replies not yet sent, and
// when nothing is left, `wakes` — then a verdict, "end" or "stall".
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
  // The host resumes what `woke` frames name. (False: stage 1's mistake.)
  const HEEDS_WOKE: bool
  // A closed frame drops the reply the host had queued. (False: the Python
  // host's mistake, fixed in the deadline work.)
  const DROPS_CLOSED: bool

  // Enough instructions to run any one call to its next wait.
  pure val STEPS = 8
  // More host actions than any instance's longest run takes.
  pure val BOUND = 60

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
  }

  // The library while a call runs, and what the call has emitted so far.
  type World = {
    machines: int -> Machine,
    queued: str -> int,
    senders: str -> int,
    woke: Set[int],
    spawned: Set[int],
    asked: Set[int],
    closed: Set[int],
    // Programs of children spawned in this call, registered when it returns.
    children: List[str],
  }

  type Event =
    | Resume(int)
    | Reply((int, int))

  // A call's output, delivered to the host when the call returns.
  type Output = {
    woke: Set[int],
    spawned: Set[int],
    asked: Set[int],
    closed: Set[int],
    status: Status,
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
  // Host actions taken so far: bounded exactly when every run terminates.
  var steps: int

  pure def fresh(prog: str): Machine = {
    prog: prog,
    pc: 0,
    status: NotStarted,
    waiting: "",
    woken: false,
    asks: Set(),
    racing: 0,
    next_id: 1,
    holds: HOLDS.get(prog),
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
      | Spawn(p) =>
        // The child's closure holds its senders already; the table registers
        // it only when this call returns.
        { ...update(w, h, mm => { ...mm, pc: mm.pc + 1 }),
          senders: HOLDS.get(p).fold(w.senders, (s, c) => s.set(c, s.get(c) + 1)),
          children: w.children.append(p) }
      | Ask =>
        { ...update(w, h, mm => { ...mm, status: Awaiting, asks: mm.asks.union(Set(mm.next_id)), next_id: mm.next_id + 1 }),
          asked: w.asked.union(Set(m.next_id)) }
      | Race(c) =>
        if (ready(w, c)) update(take(w, c), h, mm => { ...mm, pc: mm.pc + 1 })
        else
          { ...update(w, h, mm => { ...mm, status: Awaiting, waiting: c, woken: false, racing: mm.next_id,
                                   asks: mm.asks.union(Set(mm.next_id)), next_id: mm.next_id + 1 }),
            asked: w.asked.union(Set(m.next_id)) }
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
    val unspawned = w.children.foldl(Set(), (s, p) => s.union(HOLDS.get(p)))
    val gone = update(w, h, mm => { ...mm, status: Gone, asks: Set(), racing: 0, waiting: "", holds: Set() })
    drop_senders(drop_senders(gone, m.holds), unspawned)
  }

  // Register the children a successful call spawned, numbered in order.
  pure def register(w: World): World =
    w.children.foldl(w, (acc, p) => {
      val handle = acc.machines.keys().size()
      { ...acc, machines: acc.machines.put(handle, fresh(p)), spawned: acc.spawned.union(Set(handle)) }
    })

  pure def target(e: Event): int = match e { | Resume(h) => h | Reply(r) => r._1 }

  // ---- The host's actions ------------------------------------------------

  action init = all {
    machines' = Map(0 -> fresh(ROOT)),
    queued' = CHANNELS.mapBy(_ => 0),
    senders' = CHANNELS.mapBy(c => if (HOLDS.get(ROOT).contains(c)) 1 else 0),
    wakes' = Set(),
    pending' = Set(Resume(0)),
    in_flight' = Map(),
    verdict' = "running",
    stale' = 0,
    steps' = 0,
  }

  // Make a call: on a worker that is free, for a machine with no call in
  // flight. The call runs at once; its output reaches the host on `ret`.
  action call = {
    nondet e = pending.oneOf()
    val h = target(e)
    val w0: World = { machines: machines, queued: queued, senders: senders,
                      woke: Set(), spawned: Set(), asked: Set(), closed: Set(), children: [] }
    val w1 = match e { | Resume(_) => resume(w0, h) | Reply(r) => deliver(w0, h, r._2) }
    val w2 = run_to_wait(w1, h)
    val failed = w2.machines.get(h).status == Panicking
    val w3 = if (failed) fail_call(w2, h) else register(w2)
    val is_stale = match e { | Reply(r) => not(machines.get(h).asks.contains(r._2)) | _ => false }
    all {
      verdict == "running",
      not(in_flight.keys().contains(h)),
      in_flight.keys().size() < WORKERS,
      machines' = w3.machines,
      queued' = w3.queued,
      senders' = w3.senders,
      wakes' = if (failed) wakes.union(w3.woke) else wakes,
      pending' = pending.exclude(Set(e)),
      in_flight' = in_flight.put(h, {
        woke: if (failed) Set() else w3.woke,
        spawned: w3.spawned,
        asked: if (failed) Set() else w3.asked,
        closed: if (failed) Set() else w3.closed,
        status: w3.machines.get(h).status,
      }),
      verdict' = verdict,
      stale' = if (is_stale) stale + 1 else stale,
      steps' = steps + 1,
    }
  }

  // A call returns, and the host acts on its output: resume what woke,
  // begin each child, queue a reply for each ask, drop replies to what
  // closed, and free the machine if it is done.
  action ret = {
    nondet h = in_flight.keys().oneOf()
    val out = in_flight.get(h)
    val woken = if (HEEDS_WOKE) out.woke.map(m => Resume(m)) else Set()
    val dropped = if (DROPS_CLOSED) out.closed.map(id => Reply((h, id))) else Set()
    val ended = out.status == Complete or out.status == Gone
    all {
      in_flight' = in_flight.keys().exclude(Set(h)).mapBy(k => in_flight.get(k)),
      pending' = pending
        .union(woken)
        .union(out.spawned.map(c => Resume(c)))
        .union(out.asked.map(id => Reply((h, id))))
        .exclude(dropped),
      machines' = if (ended) machines.set(h, { ...machines.get(h), status: Gone }) else machines,
      queued' = queued,
      senders' = senders,
      wakes' = wakes,
      verdict' = verdict,
      stale' = stale,
      steps' = steps + 1,
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
      verdict' = if (machines.keys().forall(h => machines.get(h).status == Gone)) "end" else "stall",
    },
    machines' = machines,
    queued' = queued,
    senders' = senders,
    in_flight' = in_flight,
    stale' = stale,
    steps' = steps + 1,
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
    steps' = steps,
  }

  action step = any { call, ret, quiet, done }

  // ---- Properties --------------------------------------------------------

  // The library would let this machine progress if it were called.
  def runnable(h: int): bool = {
    val m = machines.get(h)
    val world: World = { machines: machines, queued: queued, senders: senders,
                         woke: Set(), spawned: Set(), asked: Set(), closed: Set(), children: [] }
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
      or in_flight.keys().exists(x => in_flight.get(x).woke.contains(h) or in_flight.get(x).spawned.contains(h))
  ))

  // A stall is declared only when nothing could ever progress.
  val stall_is_sound = verdict == "stall" implies machines.keys().forall(h => not(runnable(h)))

  // The end is declared only when every machine is done.
  val end_is_sound = verdict == "end" implies machines.keys().forall(h => machines.get(h).status == Gone)

  // The host never replies to a request it was told had closed.
  val no_stale_reply = stale == 0

  val safe = no_lost_wake and stall_is_sound and end_is_sound and no_stale_reply

  // Liveness, as safety over the whole state graph — checkable exhaustively,
  // and needing no fairness: until its verdict the host can always act; no
  // run is longer than BOUND (a run that could go on forever would exceed
  // it); and the verdict, when it comes, is the expected one. So every run
  // reaches the expected verdict.
  val never_stuck = verdict == "running" implies (
    in_flight.keys() != Set()
      or pending == Set()
      or pending.exists(e => not(in_flight.keys().contains(target(e))))
  )
  val terminates = steps <= BOUND
  val right_verdict = verdict == "running" or verdict == EXPECTED

  val ends_right = never_stuck and terminates and right_verdict

  // The same, as a temporal property under weak fairness: the host always
  // reaches the verdict it should. Not checked yet — Quint 0.32's TLC
  // backend hangs on it (TODO.md); `ends_right` checks it without fairness.
  temporal fair = weakFair(call, (machines, queued, senders, wakes, pending, in_flight, verdict, stale, steps))
    and weakFair(ret, (machines, queued, senders, wakes, pending, in_flight, verdict, stale, steps))
    and weakFair(quiet, (machines, queued, senders, wakes, pending, in_flight, verdict, stale, steps))

  temporal reaches_verdict = fair implies eventually(verdict == EXPECTED)
}

// ---- Instances --------------------------------------------------------------

// Two rounds of ping-pong: each round's pong is a send no effect reports —
// only a `woke` frame tells the host to resume the parent.
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
  ).*
}
