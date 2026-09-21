//! The greeter driven by a Rust host: real sleeps, a greeting table, stdin.
//!
//! This is the whole of what a host is — a loop that performs each effect
//! and replies through the handle. Fan-out needs nothing extra: replies go
//! on a queue, so a batch with two requests outstanding is handled the same
//! as a batch with one.
//!
//! ```sh
//! printf 'alice\nbob\nquit\n' | cargo run -p greeter
//! cargo run -p greeter -- --fanout   # two waits at a time
//! ```

use effect_routine::driver::Driver;
use greeter::{Effect, fanout, greeter};
use std::{
    collections::VecDeque,
    io::{self, BufRead, Write},
    thread,
};

fn main() -> io::Result<()> {
    let driver = if std::env::args().any(|a| a == "--fanout") {
        Driver::new(fanout)
    } else {
        Driver::new(greeter)
    };

    drive(driver, io::stdin().lock(), io::stdout())
}

/// Perform every effect against real IO until the routine finishes.
fn drive<R: BufRead, W: Write>(
    mut driver: Driver<Effect>,
    mut input: R,
    mut output: W,
) -> io::Result<()> {
    let mut greeted = 0;
    let mut queue: VecDeque<Effect> = driver.start().into();

    while let Some(effect) = queue.pop_front() {
        let more = match effect {
            Effect::Write(text) => {
                writeln!(output, "{text}")?;
                continue;
            }
            Effect::ReadLine(reply) => {
                let mut line = String::new();
                input.read_line(&mut line)?;
                let line = line.trim_end().to_owned();
                // End of input ends the conversation.
                let line = if line.is_empty() {
                    String::from("quit")
                } else {
                    line
                };
                driver.reply(reply, line)
            }
            Effect::Lookup(name, reply) => driver.reply(reply, lookup(&name)),
            Effect::Sleep(after, reply) => {
                thread::sleep(after);
                driver.reply(reply, ())
            }
            Effect::Count(reply) => {
                greeted += 1;
                driver.reply(reply, greeted)
            }
        };
        queue.extend(more);
    }

    Ok(())
}

fn lookup(name: &str) -> String {
    match name {
        "alice" => "Hello",
        "bob" => "Hi",
        "carol" => "Hey",
        _ => "Greetings",
    }
    .to_owned()
}
