//! A newtype over `Ctx` gets every stdlib effect trait from `AsCtx`, and can
//! reify an effect trait its own crate adds — here, one the stdlib knows
//! nothing about.

#![expect(clippy::panic, reason = "let-else arms name the batch they expected")]

use core::{future::Future, ops::ControlFlow, time::Duration};
use sans_effort_core::{
    driver::{Driver, status::Status},
    step::Step,
};
use sans_effort_effects::{
    ask::{Ask, Asked},
    console::{WriteLine, effect::WriteLine as WriteLineEffect},
    ctx::{AsCtx, Ctx},
    time::{Sleep, effect::Sleep as SleepEffect},
};

/// An effect trait the stdlib does not have.
trait Locate {
    fn locate(&self) -> impl Future<Output = String> + Send;
}

struct Where;

impl Ask for Where {
    type Reply = String;
}

/// The newtype: one `AsCtx` impl, and the local effect trait on top.
struct HostCtx<E>(Ctx<E>);

impl<E> AsCtx for HostCtx<E> {
    type Vocabulary = E;

    fn ctx(&self) -> &Ctx<E> {
        &self.0
    }
}

impl<E: From<Asked<Where>> + Send> Locate for HostCtx<E> {
    async fn locate(&self) -> String {
        self.ctx().ask(Where).await
    }
}

enum Effect {
    Sleep(Asked<SleepEffect>),
    WriteLine(WriteLineEffect),
    Where(Asked<Where>),
}

impl From<Asked<SleepEffect>> for Effect {
    fn from(asked: Asked<SleepEffect>) -> Self {
        Effect::Sleep(asked)
    }
}

impl From<WriteLineEffect> for Effect {
    fn from(write: WriteLineEffect) -> Self {
        Effect::WriteLine(write)
    }
}

impl From<Asked<Where>> for Effect {
    fn from(asked: Asked<Where>) -> Self {
        Effect::Where(asked)
    }
}

/// Uses two stdlib effect traits and the local one.
struct Postcard<C>(C);

impl<C: Locate + Sleep + WriteLine> Step for Postcard<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        let here = self.0.locate().await;
        self.0.sleep(Duration::from_millis(5)).await;
        self.0.write_line(format!("greetings from {here}"));
        ControlFlow::Break(())
    }
}

#[test]
fn a_newtype_gets_the_stdlib_and_adds_its_own() {
    let mut driver = Driver::<Effect>::new(|outbox| Postcard(HostCtx(Ctx::new(outbox))).run());

    let Some(Effect::Where(Asked { reply, .. })) = driver.resume().into_iter().next() else {
        panic!("the routine locates first");
    };
    let Some(Effect::Sleep(Asked {
        request,
        reply: wake,
    })) = driver
        .reply(reply, String::from("Nantes"))
        .into_iter()
        .next()
    else {
        panic!("then sleeps");
    };
    assert_eq!(request, SleepEffect(Duration::from_millis(5)));

    let written: Vec<String> = driver
        .reply(wake, ())
        .into_iter()
        .filter_map(|e| match e {
            Effect::WriteLine(WriteLineEffect(line)) => Some(line),
            Effect::Sleep(_) | Effect::Where(_) => None,
        })
        .collect();
    assert_eq!(written, ["greetings from Nantes"]);
    assert_eq!(driver.status(), Status::Complete);
}

/// Effect traits follow through references and smart pointers to any
/// `AsCtx`, so a context can be lent or shared. Not through `Rc`: it is never
/// `Sync`, and an effect trait's future borrows its context across threads.
#[test]
fn references_and_smart_pointers_forward() {
    fn has_both<C: Sleep + WriteLine>(_: &C) {}

    drop(Driver::<Effect>::new(|outbox| {
        let ctx = HostCtx(Ctx::new(outbox));
        has_both(&&ctx);
        let boxed: Box<HostCtx<Effect>> = Box::new(ctx);
        has_both(&boxed);
        has_both(&std::sync::Arc::new(boxed));
        async {}
    }));
}
