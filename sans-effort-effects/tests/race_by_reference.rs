//! Racing a read by reference, as a host sees it: the deadline passes, the
//! sleep's request is answered and the routine goes on waiting — but the
//! read's request is never closed. Only the sleep the routine stopped needing
//! is; the read is answered late, and nothing is lost.

use core::{pin::pin, time::Duration};
use sans_effort_core::{
    driver::{Driver, status::Status},
    select::{Either, select},
};
use sans_effort_effects::{
    ask::Asked,
    console::{ReadLine, ReadLineEffect, ReadLineError, WriteLine, WriteLineEffect},
    ctx::Ctx,
    time::{Sleep, SleepEffect},
};
use testresult::TestResult;

enum Effect {
    ReadLine(Asked<ReadLineEffect>),
    Sleep(Asked<SleepEffect>),
    WriteLine(WriteLineEffect),
}

impl From<Asked<ReadLineEffect>> for Effect {
    fn from(asked: Asked<ReadLineEffect>) -> Self {
        Self::ReadLine(asked)
    }
}

impl From<Asked<SleepEffect>> for Effect {
    fn from(asked: Asked<SleepEffect>) -> Self {
        Self::Sleep(asked)
    }
}

impl From<WriteLineEffect> for Effect {
    fn from(line: WriteLineEffect) -> Self {
        Self::WriteLine(line)
    }
}

/// `ReadLine`'s documented pattern: the next line, with a reminder at every
/// deadline until it comes.
async fn patiently<C: ReadLine + Sleep + WriteLine>(ctx: &C) -> Result<String, ReadLineError> {
    let mut read = pin!(ctx.read_line());
    loop {
        match select(read.as_mut(), ctx.sleep(Duration::from_secs(5))).await {
            Either::Left(line) => return line,
            Either::Right(()) => ctx.write_line("Still waiting…".into()),
        }
    }
}

#[test]
fn a_read_raced_by_reference_outlives_its_deadlines() -> TestResult {
    let mut driver = Driver::new(|outbox| async move {
        let ctx = Ctx::<Effect>::new(outbox);
        let line = patiently(&ctx).await;
        ctx.write_line(format!("got {line:?}"));
    });

    let (first, closed) = driver.resume().into_parts();
    let [
        Effect::ReadLine(Asked { reply: read, .. }),
        Effect::Sleep(Asked { reply: sleep, .. }),
    ] = <[Effect; 2]>::try_from(first).map_err(|_| "a read and a deadline")?
    else {
        return Err("first batch: read, then sleep")?;
    };
    assert!(closed.is_empty());

    // Two deadlines pass. Each is answered; the routine reminds, and sets
    // the next — and the read stays open throughout.
    let mut sleep = sleep;
    for _ in 0..2 {
        let (batch, closed) = driver.reply(sleep, ()).into_parts();
        let [
            Effect::WriteLine(WriteLineEffect(reminder)),
            Effect::Sleep(Asked { reply: next, .. }),
        ] = <[Effect; 2]>::try_from(batch).map_err(|_| "a reminder and a deadline")?
        else {
            return Err("after a deadline: remind, then sleep again")?;
        };
        assert_eq!(reminder, "Still waiting…");
        assert!(closed.is_empty(), "the read was not abandoned");
        sleep = next;
    }

    // The line comes. The read wins this time, and only the pending
    // deadline — the one the routine no longer needs — closes.
    let pending = sleep.id();
    let (last, closed) = driver.reply(read, Ok("alice".into())).into_parts();
    let [Effect::WriteLine(WriteLineEffect(got))] =
        <[Effect; 1]>::try_from(last).map_err(|_| "the line, written")?
    else {
        return Err("last batch: the line")?;
    };
    assert_eq!(got, r#"got Ok("alice")"#);
    assert_eq!(closed, [pending]);
    assert_eq!(driver.status(), Status::Complete);
    Ok(())
}
