//! Each reifying effect-trait impl asks for exactly its request and returns
//! exactly the host's answer, whatever both are.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert their preconditions; let-else arms name the effect they expected"
)]

use core::{future::Future, time::Duration};
use sans_effort_core::driver::{Driver, status::Status};
use sans_effort_effects::{
    ask::Asked,
    ctx::Ctx,
    env::{Var, VarEffect},
    fs::{FsError, ReadFile, ReadFileEffect, WriteFile, WriteFileEffect},
    random::{Random, RandomEffect},
    time::{Now, NowEffect, UnixTime},
};
use std::sync::{Arc, Mutex};

enum Effect {
    Now(Asked<NowEffect>),
    Random(Asked<RandomEffect>),
    ReadFile(Asked<ReadFileEffect>),
    Var(Asked<VarEffect>),
    WriteFile(Asked<WriteFileEffect>),
}

impl From<Asked<NowEffect>> for Effect {
    fn from(asked: Asked<NowEffect>) -> Self {
        Self::Now(asked)
    }
}

impl From<Asked<RandomEffect>> for Effect {
    fn from(asked: Asked<RandomEffect>) -> Self {
        Self::Random(asked)
    }
}

impl From<Asked<ReadFileEffect>> for Effect {
    fn from(asked: Asked<ReadFileEffect>) -> Self {
        Self::ReadFile(asked)
    }
}

impl From<Asked<VarEffect>> for Effect {
    fn from(asked: Asked<VarEffect>) -> Self {
        Self::Var(asked)
    }
}

impl From<Asked<WriteFileEffect>> for Effect {
    fn from(asked: Asked<WriteFileEffect>) -> Self {
        Self::WriteFile(asked)
    }
}

/// What the routine got back from its one call.
type Got<T> = Arc<Mutex<Option<T>>>;

/// A machine whose routine makes one call through a reifying context and
/// keeps what it got; its one effect; and where the result will be.
fn one_call<T: Send + 'static, Fut: Future<Output = T> + Send + 'static>(
    call: impl FnOnce(Ctx<Effect>) -> Fut + Send + 'static,
) -> (Driver<Effect>, Effect, Got<T>) {
    let got = Got::default();
    let kept = Arc::clone(&got);
    let mut driver = Driver::new(move |outbox| async move {
        let value = call(Ctx::new(outbox)).await;
        *kept.lock().expect("unpoisoned") = Some(value);
    });
    let mut effects = driver.resume().into_iter();
    let effect = effects.next().expect("one ask");
    assert!(effects.next().is_none(), "only one");
    (driver, effect, got)
}

/// After the reply: the routine finished, with `expected`.
fn finished_with<T: PartialEq + core::fmt::Debug>(
    driver: &Driver<Effect>,
    got: &Got<T>,
    expected: T,
) {
    assert_eq!(driver.status(), Status::Complete);
    assert_eq!(got.lock().expect("unpoisoned").take(), Some(expected));
}

/// An answer for a file request, from a generated choice.
fn fs_result<T>(choice: u8, ok: T) -> Result<T, FsError> {
    match choice % 4 {
        0 => Ok(ok),
        1 => Err(FsError::NotFound),
        2 => Err(FsError::PermissionDenied),
        _ => Err(FsError::Other),
    }
}

#[test]
fn var_asks_for_its_name_and_returns_the_answer() {
    bolero::check!()
        .with_type::<(String, Option<String>)>()
        .for_each(|(name, value)| {
            let asked = name.clone();
            let (mut driver, effect, got) =
                one_call(move |ctx| async move { ctx.var(asked).await });
            let Effect::Var(Asked {
                request: VarEffect(requested),
                reply,
            }) = effect
            else {
                panic!("a Var");
            };
            assert_eq!(&requested, name);
            drop(driver.reply(reply, value.clone()));
            finished_with(&driver, &got, value.clone());
        });
}

#[test]
fn read_file_asks_for_its_path_and_returns_the_answer() {
    bolero::check!()
        .with_type::<(String, Vec<u8>, u8)>()
        .for_each(|(path, bytes, choice)| {
            let asked = path.clone();
            let (mut driver, effect, got) =
                one_call(move |ctx| async move { ctx.read_file(asked).await });
            let Effect::ReadFile(Asked {
                request: ReadFileEffect(requested),
                reply,
            }) = effect
            else {
                panic!("a ReadFile");
            };
            assert_eq!(&requested, path);
            let answer = fs_result(*choice, bytes.clone());
            drop(driver.reply(reply, answer.clone()));
            finished_with(&driver, &got, answer);
        });
}

#[test]
fn write_file_asks_with_its_path_and_bytes_and_returns_the_answer() {
    bolero::check!()
        .with_type::<(String, Vec<u8>, u8)>()
        .for_each(|(path, bytes, choice)| {
            let (asked, written) = (path.clone(), bytes.clone());
            let (mut driver, effect, got) =
                one_call(move |ctx| async move { ctx.write_file(asked, written).await });
            let Effect::WriteFile(Asked { request, reply }) = effect else {
                panic!("a WriteFile");
            };
            assert_eq!(
                request,
                WriteFileEffect {
                    path: path.clone(),
                    bytes: bytes.clone()
                }
            );
            let answer = fs_result(*choice, ());
            drop(driver.reply(reply, answer));
            finished_with(&driver, &got, answer);
        });
}

#[test]
fn now_returns_the_answer() {
    bolero::check!().with_type::<u64>().for_each(|nanos| {
        let (mut driver, effect, got) = one_call(|ctx| async move { ctx.now().await });
        let Effect::Now(Asked { reply, .. }) = effect else {
            panic!("a Now");
        };
        let time = UnixTime::from_since_epoch(Duration::from_nanos(*nanos));
        drop(driver.reply(reply, time));
        finished_with(&driver, &got, time);
    });
}

#[test]
fn random_asks_for_its_length_and_returns_the_answer() {
    bolero::check!()
        .with_type::<(u32, Vec<u8>)>()
        .for_each(|(len, bytes)| {
            let asked = *len;
            let (mut driver, effect, got) =
                one_call(move |ctx| async move { ctx.random_bytes(asked).await });
            let Effect::Random(Asked {
                request: RandomEffect(requested),
                reply,
            }) = effect
            else {
                panic!("a Random");
            };
            assert_eq!(requested, *len);
            drop(driver.reply(reply, bytes.clone()));
            finished_with(&driver, &got, bytes.clone());
        });
}
