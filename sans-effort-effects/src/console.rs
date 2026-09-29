//! The console: reading and writing lines.

use crate::{
    ask::{Ask, Asked},
    ctx::AsCtx,
};
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
    C::Vocabulary: From<Asked<ReadLineEffect>> + Send,
{
    /// A reply that does not decode is a host bug the routine cannot report,
    /// so it reads as [`ReadLineError::Failed`].
    async fn read_line(&self) -> Result<String, ReadLineError> {
        self.ctx().ask(ReadLineEffect).await
    }
}

impl<C: AsCtx> WriteLine for C
where
    C::Vocabulary: From<WriteLineEffect>,
{
    fn write_line(&self, line: String) {
        self.ctx().tell(WriteLineEffect(line));
    }
}

/// The next line of input. Awaits a `Result<String, ReadLineError>`, which
/// crosses as `bytes`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadLineEffect;

impl Ask for ReadLineEffect {
    type Reply = Result<String, ReadLineError>;
}

/// No fields.
impl Encode for ReadLineEffect {
    fn encode(&self, _: &mut Writer) {}
}

impl Decode for ReadLineEffect {
    fn decode(_: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(ReadLineEffect)
    }
}

/// Show a line. Fire-and-forget; not a [`Ask`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteLineEffect(pub String);

/// The line, as a `str`.
impl Encode for WriteLineEffect {
    fn encode(&self, w: &mut Writer) {
        w.str(&self.0);
    }
}

impl Decode for WriteLineEffect {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.str().map(WriteLineEffect)
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
        ReadLine(Asked<ReadLineEffect>),
        WriteLine(WriteLineEffect),
    }

    impl From<Asked<ReadLineEffect>> for Effect {
        fn from(asked: Asked<ReadLineEffect>) -> Self {
            Effect::ReadLine(asked)
        }
    }

    impl From<WriteLineEffect> for Effect {
        fn from(write: WriteLineEffect) -> Self {
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
                Effect::WriteLine(WriteLineEffect(line)) => Some(line),
                Effect::ReadLine(_) => None,
            })
            .expect("the routine writes once");
        assert_eq!(driver.status(), Status::Complete);
        out
    }

    /// What `ReadOnce` writes for an answer: the line, or the error.
    fn shown(answer: &Result<String, ReadLineError>) -> String {
        match answer {
            Ok(line) => line.clone(),
            Err(e) => alloc::format!("{e}"),
        }
    }

    /// A line, a closed input, or a failed one, from a generated choice.
    fn answer(kind: u8, line: &str) -> Result<String, ReadLineError> {
        match kind % 3 {
            0 => Ok(line.into()),
            1 => Err(ReadLineError::Closed),
            _ => Err(ReadLineError::Failed),
        }
    }

    #[test]
    fn any_answer_arrives_as_itself() {
        bolero::check!()
            .with_type::<(u8, String)>()
            .for_each(|(kind, line)| {
                let reply = answer(*kind, line);
                assert_eq!(written(reply.clone()), shown(&reply));
            });
    }

    /// A host replying over an ABI holds the wire kind, `bytes`. Whatever
    /// bytes it sends, they reach the routine only if they are an encoded
    /// `Result<String, ReadLineError>`; otherwise the reply is refused, the
    /// handle comes back, and the request stays open for a good reply.
    #[test]
    fn any_bytes_are_delivered_if_they_decode_and_refused_otherwise() {
        bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
            let mut driver = Driver::<Effect>::new(|outbox| ReadOnce(Ctx::new(outbox)).run());
            let Some(Effect::ReadLine(Asked { reply, .. })) = driver.resume().into_iter().next()
            else {
                unreachable!("the routine reads first");
            };
            let Ok(wire) = Vec::<u8>::from_pending(Answer::pending(reply)) else {
                unreachable!("a fallible read crosses as bytes");
            };

            let decoded = Result::<String, ReadLineError>::from_bytes(bytes);
            let (outcome, expected) = if let Ok(decoded) = decoded {
                (driver.try_reply(wire, bytes.clone()), shown(&decoded))
            } else {
                let Err(refused) = driver.try_reply(wire, bytes.clone()) else {
                    panic!("undecodable bytes were delivered");
                };
                assert_eq!(driver.status(), Status::Awaiting, "still waiting");
                let (wire, _) = refused.into_parts();
                let good = Ok::<String, ReadLineError>("hi".into()).to_bytes();
                (driver.try_reply(wire, good), String::from("hi"))
            };

            let Ok(written) = outcome else {
                panic!("an encoded result is accepted");
            };
            let lines: Vec<String> = written
                .into_iter()
                .filter_map(|e| match e {
                    Effect::WriteLine(WriteLineEffect(line)) => Some(line),
                    Effect::ReadLine(_) => None,
                })
                .collect();
            assert_eq!(lines, [expected]);
            assert_eq!(driver.status(), Status::Complete);
        });
    }

    #[test]
    fn errors_round_trip() {
        for e in [ReadLineError::Closed, ReadLineError::Failed] {
            assert_eq!(ReadLineError::from_bytes(&e.to_bytes()), Ok(e));
        }
    }

    #[test]
    fn unknown_error_tags_are_refused() {
        for tag in 2..=u8::MAX {
            assert_eq!(
                ReadLineError::from_bytes(&[tag]),
                Err(DecodeError::UnknownTag { tag })
            );
        }
    }
}
