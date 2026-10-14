//! Writing an effect: one effect trait, from the trait to the bytes a host
//! reads.
//!
//! The standard effect traits cover what most routines need. An application
//! adds its own the same way they are written, and this guide writes one:
//! `Lookup`, the greeting word for a name — the demo's, in full. Nothing here
//! is special to this crate; every piece below is public API.
//!
//! # The Trait
//!
//! One verb, one method. The method returns `impl Future<Output = T> +
//! Send`; implementors write `async fn`. Declaring `Send` is what lets a
//! routine generic over its context spawn children that are `Send` (see
//! [the crate docs](crate#spelling)). Say what abandoning the request costs,
//! in a _Cancellation_ section: here, nothing.
//!
//! ```
//! use core::future::Future;
//!
//! /// Look up the greeting word for a name.
//! ///
//! /// # Cancellation
//! ///
//! /// _Retractable:_ an abandoned lookup is discarded, and loses nothing.
//! pub trait Lookup {
//!     /// The greeting word for `name`.
//!     fn lookup(&self, name: String) -> impl Future<Output = String> + Send;
//! }
//! ```
//!
//! # The Effect
//!
//! Under a reifying context, each call becomes a request value. Name it after
//! the trait, plus `Effect`, and say what answers it through
//! [`Ask`](crate::ask::Ask): an [`Answer`](sans_effort_core::reply::Answer),
//! which is one of the four wire kinds (`String`, `u64`, `()`, `Vec<u8>`) or
//! a type that crosses as one — `Result` and `Option` cross as bytes. A trait
//! that can fail for reasons outside the routine answers with a `Result`;
//! one that cannot, with the value itself.
//!
//! ```
//! use sans_effort_effects::ask::Ask;
//!
//! /// What `Lookup` records under a reifying context. Awaits a `str`.
//! #[derive(Debug, Clone, PartialEq, Eq)]
//! pub struct LookupEffect(pub String);
//!
//! impl Ask for LookupEffect {
//!     type Reply = String;
//! }
//! ```
//!
//! A trait whose calls need no answer — a message, like
//! [`WriteLine`](crate::console::WriteLine) — has an effect struct that does
//! not implement `Ask`, and is _told_ rather than asked.
//!
//! # The Reifying Impl
//!
//! One blanket impl makes every reifying context implement the trait,
//! whatever the host's vocabulary — provided the vocabulary can carry the
//! effect:
//!
//! ```
//! # use core::future::Future;
//! # pub trait Lookup { fn lookup(&self, name: String) -> impl Future<Output = String> + Send; }
//! # #[derive(Debug, Clone, PartialEq, Eq)] pub struct LookupEffect(pub String);
//! # impl sans_effort_effects::ask::Ask for LookupEffect { type Reply = String; }
//! use sans_effort_effects::{ask::Asked, ctx::AsCtx};
//!
//! impl<C: AsCtx + Sync> Lookup for C
//! where
//!     C::Vocabulary: From<Asked<LookupEffect>> + Send,
//! {
//!     async fn lookup(&self, name: String) -> String {
//!         self.ctx().ask(LookupEffect(name)).await
//!     }
//! }
//! ```
//!
//! Each bound has a reason:
//!
//! - _`C: AsCtx`_ rather than `Ctx<E>`: a newtype over [`Ctx`](crate::ctx::Ctx)
//!   that implements [`AsCtx`](crate::ctx::AsCtx) gets the trait too, which
//!   is how a crate reifies an effect trait it does not own.
//! - _`C: Sync`:_ the future borrows `&self` across its `.await`, and a
//!   `&C` is `Send` only if `C` is `Sync`.
//! - _`C::Vocabulary: From<Asked<LookupEffect>>`:_ the requirement. The host
//!   meets it with a `From` impl; a host whose vocabulary lacks the effect
//!   does not get the trait, and a routine that needs it fails to build
//!   against that host.
//! - _`C::Vocabulary: Send`:_ the future holds the context's outbox, which is
//!   `Send` only if the effects in it are.
//!
//! A trait declared without `Send` on its future drops `C: Sync` and the
//! vocabulary's `Send`. A told effect calls
//! `self.ctx().tell(…)` instead, bounded on `From<TheEffect>` rather than
//! `From<Asked<TheEffect>>`.
//!
//! # Native Impls
//!
//! A native context implements the trait with a real future, directly, and
//! does nothing until that future is polled — `async fn` sees to it — so a
//! routine behaves the same under it as under a reifying context. For
//! one built on `sans-effort-tokio`, that is a type of the application's
//! own, wrapping a `TokioCtx` and forwarding the standard traits — the
//! orphan rule requires the impls to live on a type the application owns:
//!
//! ```
//! # use core::future::Future;
//! # pub trait Lookup { fn lookup(&self, name: String) -> impl Future<Output = String> + Send; }
//! /// A native context: the greetings an application knows.
//! struct Greetings;
//!
//! impl Lookup for Greetings {
//!     async fn lookup(&self, name: String) -> String {
//!         String::from(if name == "alice" { "Hello" } else { "Hi" })
//!     }
//! }
//! ```
//!
//! # Offering It to a Host
//!
//! A host that offers the effect adds a variant for it to its vocabulary,
//! with the `From` impl the requirement names. A foreign host cannot hold
//! a `ReplyHandle`, so the vocabulary also says how each effect crosses:
//! [`HostEffect::split`](sans_effort_core::boundary::host_effect::HostEffect::split)
//! turns an effect into its _view_ — the same effect with its handle
//! replaced by the request id — and the handle to keep, as a
//! [`Pending`](sans_effort_core::boundary::pending::Pending);
//! [`Encode`](sans_effort_core::boundary::codec::Encode) writes the view as
//! the record a host reads. The first byte of each record is its _tag_,
//! which the routine's author documents for host authors.
//!
//! Number the tags with a `#[repr(u8)]` enum rather than literals scattered
//! through `encode`: then a repeated tag is a compile error.
//!
//! ```
//! # use core::{future::Future, ops::ControlFlow};
//! # pub trait Lookup { fn lookup(&self, name: String) -> impl Future<Output = String> + Send; }
//! # #[derive(Debug, Clone, PartialEq, Eq)] pub struct LookupEffect(pub String);
//! # impl sans_effort_effects::ask::Ask for LookupEffect { type Reply = String; }
//! # use sans_effort_effects::{ask::Asked, ctx::AsCtx};
//! # impl<C: AsCtx + Sync> Lookup for C where C::Vocabulary: From<Asked<LookupEffect>> + Send {
//! #     async fn lookup(&self, name: String) -> String { self.ctx().ask(LookupEffect(name)).await }
//! # }
//! use sans_effort_core::{
//!     boundary::{
//!         codec::{Encode, Writer},
//!         host_effect::HostEffect,
//!         pending::Pending,
//!     },
//!     driver::{Driver, status::Status},
//!     reply::Answer,
//!     step::Step,
//! };
//! use sans_effort_effects::{
//!     console::{WriteLine, WriteLineEffect},
//!     ctx::Ctx,
//! };
//!
//! // The host's vocabulary: the effects it offers.
//! enum Effect {
//!     Lookup(Asked<LookupEffect>),
//!     WriteLine(WriteLineEffect),
//! }
//!
//! impl From<Asked<LookupEffect>> for Effect {
//!     fn from(asked: Asked<LookupEffect>) -> Self {
//!         Effect::Lookup(asked)
//!     }
//! }
//!
//! impl From<WriteLineEffect> for Effect {
//!     fn from(write: WriteLineEffect) -> Self {
//!         Effect::WriteLine(write)
//!     }
//! }
//!
//! // What a foreign host sees: the request id instead of the handle.
//! #[derive(Debug, PartialEq)]
//! enum View {
//!     Lookup { name: String, id: u64 },
//!     WriteLine { text: String },
//! }
//!
//! impl HostEffect for Effect {
//!     type View = View;
//!
//!     fn split(self) -> (View, Option<Pending>) {
//!         match self {
//!             Effect::Lookup(Asked { request: LookupEffect(name), reply }) => {
//!                 let id = reply.id();
//!                 (View::Lookup { name, id }, Some(String::pending(reply)))
//!             }
//!             Effect::WriteLine(WriteLineEffect(text)) => (View::WriteLine { text }, None),
//!         }
//!     }
//! }
//!
//! // The tag table, numbered once.
//! #[repr(u8)]
//! enum Tag {
//!     Lookup = 2,
//!     WriteLine = 5,
//! }
//!
//! impl Encode for View {
//!     fn encode(&self, w: &mut Writer) {
//!         match self {
//!             View::Lookup { name, id } => {
//!                 w.u8(Tag::Lookup as u8);
//!                 w.str(name);
//!                 w.u64(*id); // an ask's id comes last
//!             }
//!             View::WriteLine { text } => {
//!                 w.u8(Tag::WriteLine as u8);
//!                 w.str(text);
//!             }
//!         }
//!     }
//! }
//!
//! // A routine that uses it, driven as a host would drive it.
//! struct Greet<C>(C);
//!
//! impl<C: Lookup + WriteLine> Step for Greet<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         let word = self.0.lookup("alice".into()).await;
//!         self.0.write_line(format!("{word}, alice!"));
//!         ControlFlow::Break(())
//!     }
//! }
//!
//! let mut driver = Driver::<Effect>::new(|outbox| Greet(Ctx::new(outbox)).run());
//! let mut effects = driver.resume().into_iter();
//! let (view, pending) = effects.next().ok_or("one ask")?.split();
//! assert_eq!(view, View::Lookup { name: "alice".into(), id: 1 });
//! // Tag 2, the name as `u32 len · bytes`, then the id.
//! let record = [&[2, 5, 0, 0, 0][..], b"alice", &1_u64.to_le_bytes()].concat();
//! assert_eq!(view.to_bytes(), record);
//!
//! let Some(Pending::Str(reply)) = pending else { unreachable!("a lookup awaits a str") };
//! let (view, _) = driver.reply(reply, "Hello".into()).into_iter().next().ok_or("one tell")?.split();
//! assert_eq!(view, View::WriteLine { text: "Hello, alice!".into() });
//! assert_eq!(driver.status(), Status::Complete);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Two variants given the same tag no longer compile:
//!
//! ```compile_fail,E0081
//! #[repr(u8)]
//! enum Tag {
//!     Lookup = 2,
//!     WriteLine = 2,
//! }
//! ```
//!
//! A host that answers over the C ABI replies with a record — kind `1`
//! (`str`), the id, the string — and the host layer (`sans-effort-host`)
//! checks the kind against the `Pending` before the routine sees it.
//! `ABI.md` in the repository has the frame and record formats in full.
//!
//! # Effects That Can Fail
//!
//! A trait that can fail for reasons outside the routine — a file system, a
//! lookup that may know no answer — answers with a `Result`, and its error
//! type is part of the wire. Give the error [`Encode`] and [`Decode`], a tag
//! byte per case, and the answer crosses as `bytes`: `00 · value` for `Ok`,
//! `01 · error` for `Err` (an `Option` likewise: `00` for none, `01 · value`).
//! A reply is checked against the `Result` when it is delivered, before the
//! routine sees it: bytes that do not decode are refused — `MALFORMED` over
//! the C ABI, a [`Refused`](sans_effort_core::driver::Refused) from
//! `try_reply` in Rust — and the request stays open for a good reply.
//!
//! [`Encode`]: sans_effort_core::boundary::codec::Encode
//! [`Decode`]: sans_effort_core::boundary::codec::Decode
//!
//! ```
//! use core::{future::Future, ops::ControlFlow};
//! use sans_effort_core::{
//!     boundary::{
//!         codec::{Decode, DecodeError, Encode, Reader, Writer},
//!         pending::Pending,
//!     },
//!     driver::Driver,
//!     reply::Answer,
//!     step::Step,
//! };
//! use sans_effort_effects::{
//!     ask::{Ask, Asked},
//!     console::{WriteLine, WriteLineEffect},
//!     ctx::{AsCtx, Ctx},
//! };
//!
//! /// Why there is no greeting.
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! pub enum GreetingError {
//!     /// No greeting is known for the name.
//!     Unknown,
//! }
//!
//! /// `Unknown` is tag `0`.
//! impl Encode for GreetingError {
//!     fn encode(&self, w: &mut Writer) {
//!         match self {
//!             GreetingError::Unknown => w.u8(0),
//!         }
//!     }
//! }
//!
//! impl Decode for GreetingError {
//!     fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
//!         match r.u8()? {
//!             0 => Ok(GreetingError::Unknown),
//!             tag => Err(DecodeError::UnknownTag { tag }),
//!         }
//!     }
//! }
//!
//! /// The greeting word for a name, if there is one.
//! pub trait Greeting {
//!     /// The greeting word for `name`.
//!     fn greeting(&self, name: String)
//!         -> impl Future<Output = Result<String, GreetingError>> + Send;
//! }
//!
//! /// What `Greeting` records. Awaits `bytes`: an encoded `Result`.
//! pub struct GreetingEffect(pub String);
//!
//! impl Ask for GreetingEffect {
//!     type Reply = Result<String, GreetingError>;
//! }
//!
//! impl<C: AsCtx + Sync> Greeting for C
//! where
//!     C::Vocabulary: From<Asked<GreetingEffect>> + Send,
//! {
//!     async fn greeting(&self, name: String) -> Result<String, GreetingError> {
//!         self.ctx().ask(GreetingEffect(name)).await
//!     }
//! }
//!
//! enum Effect {
//!     Greeting(Asked<GreetingEffect>),
//!     WriteLine(WriteLineEffect),
//! }
//!
//! impl From<Asked<GreetingEffect>> for Effect {
//!     fn from(asked: Asked<GreetingEffect>) -> Self {
//!         Effect::Greeting(asked)
//!     }
//! }
//!
//! impl From<WriteLineEffect> for Effect {
//!     fn from(write: WriteLineEffect) -> Self {
//!         Effect::WriteLine(write)
//!     }
//! }
//!
//! struct Greet<C>(C);
//!
//! impl<C: Greeting + WriteLine> Step for Greet<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         match self.0.greeting("bob".into()).await {
//!             Ok(word) => self.0.write_line(format!("{word}, bob!")),
//!             Err(GreetingError::Unknown) => self.0.write_line("Who is bob?".into()),
//!         }
//!         ControlFlow::Break(())
//!     }
//! }
//!
//! let mut driver = Driver::<Effect>::new(|outbox| Greet(Ctx::new(outbox)).run());
//! let Some(Effect::Greeting(Asked { reply, .. })) = driver.resume().into_iter().next() else {
//!     unreachable!("it asks first")
//! };
//!
//! // On the wire, a `Result` is a `bytes` reply.
//! let Pending::Bytes(reply) = <Result<String, GreetingError>>::pending(reply) else {
//!     unreachable!("a `Result` crosses as bytes")
//! };
//!
//! // Not an encoded `Result<String, GreetingError>`: refused, and still open.
//! let (reply, _why) = driver.try_reply(reply, vec![7]).err().ok_or("refused")?.into_parts();
//!
//! // `01 · 00`: `Err(GreetingError::Unknown)`.
//! let told = driver.try_reply(reply, vec![1, 0])?;
//! let Some(Effect::WriteLine(WriteLineEffect(text))) = told.into_iter().next() else {
//!     unreachable!("then it writes")
//! };
//! assert_eq!(text, "Who is bob?");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Testing It
//!
//! An effect trait wants two kinds of test. The routine's _logic_ runs
//! against a native impl, like `Greetings` above, in an ordinary async test.
//! The _effect_ (what the routine asks for, and what it does with each
//! answer) runs under a reifying context with `sans_effort::testing::drive`,
//! behind the `testing` feature. It drives the routine and every machine the
//! routine spawns, calling a handler for each effect, which answers asks
//! through `Answers` and records what was told. `drive_with` takes a
//! schedule, such as `Choices` from a fuzzer's bytes, to try the other orders
//! a host may choose.
//!
//! ```
//! # use core::{future::Future, ops::ControlFlow};
//! # pub trait Lookup { fn lookup(&self, name: String) -> impl Future<Output = String> + Send; }
//! # #[derive(Debug, Clone, PartialEq, Eq)] pub struct LookupEffect(pub String);
//! # impl sans_effort_effects::ask::Ask for LookupEffect { type Reply = String; }
//! # use sans_effort_effects::{ask::Asked, ctx::AsCtx};
//! # impl<C: AsCtx + Sync> Lookup for C where C::Vocabulary: From<Asked<LookupEffect>> + Send {
//! #     async fn lookup(&self, name: String) -> String { self.ctx().ask(LookupEffect(name)).await }
//! # }
//! # use sans_effort_effects::console::{WriteLine, WriteLineEffect};
//! # enum Effect { Lookup(Asked<LookupEffect>), WriteLine(WriteLineEffect) }
//! # impl From<Asked<LookupEffect>> for Effect { fn from(a: Asked<LookupEffect>) -> Self { Effect::Lookup(a) } }
//! # impl From<WriteLineEffect> for Effect { fn from(w: WriteLineEffect) -> Self { Effect::WriteLine(w) } }
//! # use sans_effort_core::step::Step;
//! # struct Greet<C>(C);
//! # impl<C: Lookup + WriteLine> Step for Greet<C> {
//! #     async fn step(&mut self) -> ControlFlow<()> {
//! #         let word = self.0.lookup("alice".into()).await;
//! #         self.0.write_line(format!("{word}, alice!"));
//! #         ControlFlow::Break(())
//! #     }
//! # }
//! use sans_effort_core::{driver::Driver, testing::drive};
//! use sans_effort_effects::ctx::Ctx;
//!
//! // `Greet` and its vocabulary `Effect`, from the example above.
//! let mut written = Vec::new();
//! drive(Driver::new(|outbox| Greet(Ctx::new(outbox)).run()), |effect, host| match effect {
//!     Effect::Lookup(Asked { request: LookupEffect(name), reply }) => {
//!         host.reply(reply, format!("Hello to {name}"));
//!     }
//!     Effect::WriteLine(WriteLineEffect(text)) => written.push(text),
//! })?;
//! assert_eq!(written, ["Hello to alice, alice!"]);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
