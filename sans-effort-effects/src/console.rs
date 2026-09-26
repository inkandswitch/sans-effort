//! The console: reading and writing lines.

pub mod effect;

use crate::{ask::Asked, ctx::AsCtx};
use alloc::string::String;
use core::future::Future;
use sans_effort_core::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};

/// Read a line.
pub trait ReadLine {
    /// The next line, without its line ending.
    ///
    /// # Errors
    ///
    /// [`ReadLineError::Closed`] at the end of input;
    /// [`ReadLineError::Failed`] if the input could not be read.
    fn read_line(&self) -> impl Future<Output = Result<String, ReadLineError>> + Send;
}

/// Write a line. Fire-and-forget, so not `async`.
pub trait WriteLine {
    /// Show `line`.
    fn write_line(&self, line: String);
}

impl<C: AsCtx + Sync> ReadLine for C
where
    C::Vocabulary: From<Asked<effect::ReadLine>> + Send,
{
    /// A reply that does not decode is a host bug the routine cannot report,
    /// so it reads as [`ReadLineError::Failed`].
    async fn read_line(&self) -> Result<String, ReadLineError> {
        self.ctx().ask(effect::ReadLine).await
    }
}

impl<C: AsCtx> WriteLine for C
where
    C::Vocabulary: From<effect::WriteLine>,
{
    fn write_line(&self, line: String) {
        self.ctx().tell(effect::WriteLine(line));
    }
}

/// Why no line was read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ReadLineError {
    /// The input has ended; no more lines will come.
    #[error("input closed")]
    Closed,

    /// The input could not be read.
    #[error("input failed")]
    Failed,
}

/// `Closed` is tag `0`, `Failed` tag `1`.
impl Encode for ReadLineError {
    fn encode(&self, w: &mut Writer) {
        w.u8(match self {
            ReadLineError::Closed => 0,
            ReadLineError::Failed => 1,
        });
    }
}

impl Decode for ReadLineError {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        match r.u8()? {
            0 => Ok(ReadLineError::Closed),
            1 => Ok(ReadLineError::Failed),
            tag => Err(DecodeError::UnknownTag { tag }),
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert their preconditions; let-else arms name the outcome they expected"
    )]

    use super::*;
    use crate::ctx::Ctx;
    use alloc::vec::Vec;
    use core::ops::ControlFlow;
    use sans_effort_core::{
        driver::{Driver, status::Status},
        reply::{Answer, Reply},
        step::Step,
    };

    enum Effect {
        ReadLine(Asked<effect::ReadLine>),
        WriteLine(effect::WriteLine),
    }

    impl From<Asked<effect::ReadLine>> for Effect {
        fn from(asked: Asked<effect::ReadLine>) -> Self {
            Effect::ReadLine(asked)
        }
    }

    impl From<effect::WriteLine> for Effect {
        fn from(write: effect::WriteLine) -> Self {
            Effect::WriteLine(write)
        }
    }

    /// Reads once and writes what it got, or the error.
    struct ReadOnce<C>(C);

    impl<C: ReadLine + WriteLine> Step for ReadOnce<C> {
        async fn step(&mut self) -> ControlFlow<()> {
            let line = match self.0.read_line().await {
                Ok(line) => line,
                Err(e) => alloc::format!("{e}"),
            };
            self.0.write_line(line);
            ControlFlow::Break(())
        }
    }

    /// What the routine wrote when the host answered its one read with `reply`.
    fn written(reply: Result<String, ReadLineError>) -> String {
        let mut driver = Driver::<Effect>::new(|outbox| ReadOnce(Ctx::new(outbox)).run());
        let Some(Effect::ReadLine(Asked { reply: handle, .. })) =
            driver.resume().into_iter().next()
        else {
            unreachable!("the routine reads first");
        };
        let out = driver
            .reply(handle, reply)
            .into_iter()
            .find_map(|e| match e {
                Effect::WriteLine(effect::WriteLine(line)) => Some(line),
                Effect::ReadLine(_) => None,
            })
            .expect("the routine writes once");
        assert_eq!(driver.status(), Status::Complete);
        out
    }

    #[test]
    fn a_line_a_closed_input_and_a_failure_arrive_as_themselves() {
        assert_eq!(written(Ok("hi".into())), "hi");
        assert_eq!(written(Err(ReadLineError::Closed)), "input closed");
        assert_eq!(written(Err(ReadLineError::Failed)), "input failed");
    }

    /// A host replying over an ABI holds the wire kind, `bytes`. Bytes that
    /// are not an encoded `Result<String, ReadLineError>` never reach the
    /// routine: the reply is refused, the handle comes back, and the request
    /// stays open for a good reply.
    #[test]
    fn bytes_that_do_not_decode_are_refused_and_the_request_stays_open() {
        let mut driver = Driver::<Effect>::new(|outbox| ReadOnce(Ctx::new(outbox)).run());
        let Some(Effect::ReadLine(Asked { reply, .. })) = driver.resume().into_iter().next() else {
            unreachable!("the routine reads first");
        };
        let Ok(wire) = Vec::<u8>::from_pending(Answer::pending(reply)) else {
            unreachable!("a fallible read crosses as bytes");
        };

        let Err(refused) = driver.try_reply(wire, alloc::vec![7]) else {
            panic!("7 is not an encoded result");
        };
        let (wire, _) = refused.into_parts();
        assert_eq!(driver.status(), Status::Awaiting, "still waiting");

        let Ok(written) =
            driver.try_reply(wire, Ok::<String, ReadLineError>("hi".into()).to_bytes())
        else {
            panic!("a good reply is accepted");
        };
        let lines: Vec<String> = written
            .into_iter()
            .filter_map(|e| match e {
                Effect::WriteLine(effect::WriteLine(line)) => Some(line),
                Effect::ReadLine(_) => None,
            })
            .collect();
        assert_eq!(lines, ["hi"]);
        assert_eq!(driver.status(), Status::Complete);
    }

    #[test]
    fn errors_round_trip() {
        for e in [ReadLineError::Closed, ReadLineError::Failed] {
            assert_eq!(ReadLineError::from_bytes(&e.to_bytes()), Ok(e));
        }
    }
}
