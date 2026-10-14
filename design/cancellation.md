# Cancellation

## `select`

`join(a, b)` waits for both. Routines often need whichever comes first instead: a receive with a timeout, a heartbeat, "stop or keep working". `no_std` has no `select!`, so `sans-effort` has a small combinator beside `join`:

```rust
match select(rx.recv(), ctx.sleep(TIMEOUT)).await {
    Either::Left(msg) => handle(msg),
    Either::Right(()) => give_up(),
}
```

Under tokio that is the whole story: dropping the losing future cancels it.

## Behind a Driver

The sleep is an ask, so the host sees it: `[(Sleep, 5)]`. The receive is not an effect; when another machine sends, the host learns of it from a `woke` frame and resumes this one. If the message arrives first, the routine drops the sleep, and the request closes:

- its slot in the driver's mailbox goes;
- its id is reported in the yield's `closed()`;
- the host layer's `Machine` forgets its reply handle;
- the call's output carries a closed frame for it, so a foreign host can stop the work.

```mermaid
sequenceDiagram
    participant H as Host
    participant R as Routine

    R-->>H: [(Sleep, 5)] · AWAITING
    Note over H: another machine sends; its output names this one in a woke frame
    H->>R: resume
    Note over R: select picks the receive; the sleep is dropped
    R-->>H: [Closed 5, …]
    Note over H: cancel the timer for 5
```

## Closed Frames

Every effect crosses as a frame, `u8 kind · u32 len · payload`, and a closed id is one more frame kind:

```text
  kind 3 · len 8 · u64 id        "the routine no longer needs a reply to id"
```

It belongs to the ABI, not to any routine's vocabulary, so it takes no tag. A host that does not know frame kind `3` skips it, as it skips every unknown kind; it simply never cancels early.

The precedent is Cap'n Proto's RPC `Finish` message ("the caller no longer needs this answer"), which lets the callee cancel. Here the routine is the caller and the host the callee, so a closed frame is `Finish`.

## Limits

### A Closed Frame Means "No Longer Needed", Not "Undone"

If `select(pay(), sleep(TIMEOUT))` times out, the payment may still go through. Cancellation tells the host it may stop; it cannot unperform an effect that has started. Tokio has the same property. A routine that races an effect with consequences must treat the losing branch as _possibly done_. Each effect says which kind it is (see [Cancel Safety](#cancel-safety)).

### Late Replies Stay Harmless

A host can still reply to an abandoned id: a timer can fire while the call that reports `Closed 5` is in flight. The reply is refused as `STALE` and changes nothing: no bytes are written, the routine is not polled, and the machine stays in the table. The codes keep the cases apart. `STALE` is a reply to an id that was issued but is no longer awaited (answered, or closed); `BAD_INPUT` is an id never issued; `MALFORMED` is a record that does not parse. So a host that has been told "5 closed" expects a refused reply to 5, and treats a refusal for any other id as a bug.

### Completion, Panics, and `free` Close Everything

A routine that completes, panics, or is freed with requests outstanding has abandoned all of them. The final batch on `COMPLETE` carries a closed frame for each. `PANICKED` and `free` close every outstanding id without one, and `ABI.md` says so. Otherwise hosts would leak work at exactly the moments that matter most.

### Only Asks Can Be Cancelled

A tell has no id and no reply, so there is nothing to close.

## Cancel Safety

`select` is correct whatever it races: the loser is dropped, its asks close, and a late reply is refused. What the race can _lose_ depends on the loser alone, so cancel safety is each effect's to state, not the mechanism's. Every effect trait has a "Cancellation" section naming one of three kinds:

| Kind          | Effects                           | If its request closes after the host performed it                                                                                                                                        |
|---------------|-----------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| _Retractable_ | `Sleep`, `Now`, `Var`, `ReadFile` | The result is discarded; nothing is lost. A host should stop the work (a timer).                                                                                                         |
| _Consuming_   | `ReadLine`; `Random`              | The value is used up. For `Random` nothing a routine needed is lost, since the next draw is as good. For `ReadLine` a line is lost: it is the one effect here that is _not cancel-safe_. |
| _Committing_  | `WriteFile`                       | It happened, or may have; the routine will not learn which.                                                                                                                              |

A host that already holds a result for a closed id discards it, and releases anything it stands for; `ABI.md` asks nothing more.

### Racing a Read Without Losing It

The pattern is tokio's: race a pinned future _by reference_. The loser dropped is then the reference, not the read, which goes on and is awaited later. No request is ever closed, so no host can lose its line:

```rust
let mut read = pin!(ctx.read_line());
loop {
    match select(read.as_mut(), ctx.sleep(Duration::from_secs(5))).await {
        Either::Left(line) => return line,
        Either::Right(()) => ctx.write_line("Still waiting…".into()),
    }
}
```

It needs no help from hosts, contexts, or the library. Its limits are visible in the routine's own code: a read still pending when the routine ends is abandoned all the same, and natively a pending read holds `TokioInput`'s lock, so another reader of a shared input waits for it.

Natively, `TokioInput` is cancel-safe on its own: it reads with `read_until` into a buffer it keeps, so a read dropped mid-line leaves its bytes for the next read. tokio's own `read_line` throws them away, so a timeout that fired while a user was typing would garble the next line.

### Alternatives

Three other ways to make an abandoned read lose nothing were tried against a worst-case host (one that takes a line as soon as it sees the ask and cannot stop a read once started):

- _Host push-back:_ a held line for a closed id goes back to the input. It loses nothing only together with a second rule (one read in flight per input, served in the order asked), and it asks every host author for something subtle. It remains the path if zero loss is ever needed.
- _A context that keeps the read alive:_ an abandoned read is parked for the next `read_line`. The parked read is per context, so a parent's late line can arrive after a child's; and natively it holds `TokioInput`'s lock with nobody polling it, which can deadlock a second reader.
- _A reader machine and a channel:_ the routine races the channel instead. It reads ahead, and loses what it read ahead when the consumer stops.

Racing by reference asks for nothing, and shares their one remaining leak: a read pending when the routine ends.
