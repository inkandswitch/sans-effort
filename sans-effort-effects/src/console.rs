//! The console: reading and writing lines.

pub mod effect;

use crate::{ctx::AsCtx, request::Asked};
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
        let bytes = self.ctx().request(effect::ReadLine).await;
        Result::<String, ReadLineError>::from_bytes(&bytes).unwrap_or(Err(ReadLineError::Failed))
    }
}

impl<C: AsCtx> WriteLine for C
where
    C::Vocabulary: From<effect::WriteLine>,
{
    fn write_line(&self, line: String) {
        self.ctx().notify(effect::WriteLine(line));
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
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use crate::ctx::Ctx;
    use alloc::vec::Vec;
    use core::ops::ControlFlow;
    use sans_effort_core::{
        driver::{Driver, status::Status},
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
    fn written(reply: Vec<u8>) -> String {
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
        assert_eq!(written(effect::ReadLine::reply(Ok("hi"))), "hi");
        assert_eq!(
            written(effect::ReadLine::reply(Err(ReadLineError::Closed))),
            "input closed"
        );
        assert_eq!(
            written(effect::ReadLine::reply(Err(ReadLineError::Failed))),
            "input failed"
        );
    }

    #[test]
    fn a_reply_that_does_not_decode_is_a_failure() {
        assert_eq!(written(alloc::vec![7]), "input failed");
    }

    #[test]
    fn the_reply_helper_matches_an_owned_result() {
        assert_eq!(
            effect::ReadLine::reply(Ok("hi")),
            Ok::<String, ReadLineError>("hi".into()).to_bytes()
        );
    }

    #[test]
    fn errors_round_trip() {
        for e in [ReadLineError::Closed, ReadLineError::Failed] {
            assert_eq!(ReadLineError::from_bytes(&e.to_bytes()), Ok(e));
        }
    }
}
