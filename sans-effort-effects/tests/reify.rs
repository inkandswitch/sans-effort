//! Each reifying effect-trait impl asks for exactly its request and returns
//! exactly the host's answer, whatever both are.

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
use std::sync::{Arc, Mutex, PoisonError};
use testresult::TestResult;

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
) -> TestResult<(Driver<Effect>, Effect, Got<T>)> {
    let got = Got::default();
    let kept = Arc::clone(&got);
    let mut driver = Driver::new(move |outbox| async move {
        let value = call(Ctx::new(outbox)).await;
        *kept.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
    });
    let effects: Vec<Effect> = driver.resume().into_iter().collect();
    let [effect] = effects
        .try_into()
        .map_err(|effects: Vec<_>| format!("one ask, not {}", effects.len()))?;
    Ok((driver, effect, got))
}

/// After the reply: the routine finished, with `expected`.
fn finished_with<T: PartialEq + core::fmt::Debug>(
    driver: &Driver<Effect>,
    got: &Got<T>,
    expected: T,
) -> TestResult {
    assert_eq!(driver.status(), Status::Complete);
    assert_eq!(got.lock()?.take(), Some(expected));
    Ok(())
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
            let Ok(()) = var_round_trip(name, value.as_deref());
        });
}

fn var_round_trip(name: &str, value: Option<&str>) -> TestResult {
    let asked = name.to_owned();
    let (mut driver, effect, got) = one_call(move |ctx| async move { ctx.var(asked).await })?;
    let Effect::Var(Asked {
        request: VarEffect(requested),
        reply,
    }) = effect
    else {
        return Err("a Var")?;
    };
    assert_eq!(requested, name);
    drop(driver.reply(reply, value.map(str::to_owned)));
    finished_with(&driver, &got, value.map(str::to_owned))
}

#[test]
fn read_file_asks_for_its_path_and_returns_the_answer() {
    bolero::check!()
        .with_type::<(String, Vec<u8>, u8)>()
        .for_each(|(path, bytes, choice)| {
            let Ok(()) = read_file_round_trip(path, bytes, *choice);
        });
}

fn read_file_round_trip(path: &str, bytes: &[u8], choice: u8) -> TestResult {
    let asked = path.to_owned();
    let (mut driver, effect, got) = one_call(move |ctx| async move { ctx.read_file(asked).await })?;
    let Effect::ReadFile(Asked {
        request: ReadFileEffect(requested),
        reply,
    }) = effect
    else {
        return Err("a ReadFile")?;
    };
    assert_eq!(requested, path);
    let answer = fs_result(choice, bytes.to_vec());
    drop(driver.reply(reply, answer.clone()));
    finished_with(&driver, &got, answer)
}

#[test]
fn write_file_asks_with_its_path_and_bytes_and_returns_the_answer() {
    bolero::check!()
        .with_type::<(String, Vec<u8>, u8)>()
        .for_each(|(path, bytes, choice)| {
            let Ok(()) = write_file_round_trip(path, bytes, *choice);
        });
}

fn write_file_round_trip(path: &str, bytes: &[u8], choice: u8) -> TestResult {
    let (asked, written) = (path.to_owned(), bytes.to_vec());
    let (mut driver, effect, got) =
        one_call(move |ctx| async move { ctx.write_file(asked, written).await })?;
    let Effect::WriteFile(Asked { request, reply }) = effect else {
        return Err("a WriteFile")?;
    };
    assert_eq!(
        request,
        WriteFileEffect {
            path: path.to_owned(),
            bytes: bytes.to_vec()
        }
    );
    let answer = fs_result(choice, ());
    drop(driver.reply(reply, answer));
    finished_with(&driver, &got, answer)
}

#[test]
fn now_returns_the_answer() {
    bolero::check!().with_type::<u64>().for_each(|nanos| {
        let Ok(()) = now_round_trip(*nanos);
    });
}

fn now_round_trip(nanos: u64) -> TestResult {
    let (mut driver, effect, got) = one_call(|ctx| async move { ctx.now().await })?;
    let Effect::Now(Asked { reply, .. }) = effect else {
        return Err("a Now")?;
    };
    let time = UnixTime::from_since_epoch(Duration::from_nanos(nanos));
    drop(driver.reply(reply, time));
    finished_with(&driver, &got, time)
}

#[test]
fn random_asks_for_its_length_and_returns_the_answer() {
    bolero::check!()
        .with_type::<(u32, Vec<u8>)>()
        .for_each(|(len, bytes)| {
            let Ok(()) = random_round_trip(*len, bytes);
        });
}

fn random_round_trip(len: u32, bytes: &[u8]) -> TestResult {
    let (mut driver, effect, got) =
        one_call(move |ctx| async move { ctx.random_bytes(len).await })?;
    let Effect::Random(Asked {
        request: RandomEffect(requested),
        reply,
    }) = effect
    else {
        return Err("a Random")?;
    };
    assert_eq!(requested, len);
    drop(driver.reply(reply, bytes.to_vec()));
    finished_with(&driver, &got, bytes.to_vec())
}
