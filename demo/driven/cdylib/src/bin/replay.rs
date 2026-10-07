//! Replay a run a foreign host recorded, in Rust.
//!
//! A host started with `--record FILE` saves its log there (through
//! `greeter_record` and `greeter_record_finish`). This repeats every call in
//! it against a fresh table, through the same root constructor, and reports
//! the first divergence — so a failing seeded run can be studied where Rust's
//! tools are.
//!
//! ```sh
//! python3 demo/driven/python/main.py --ring --seed 7 --record /tmp/ring.log
//! cargo run -p greeter_cdylib --bin replay -- /tmp/ring.log ring
//! ```
//!
//! Exact for a sequential host, such as the Python one, seeded or not. A
//! parallel host's calls race, and replaying them in log order may diverge.

use greeter_cdylib::{
    greeter_new, greeter_new_deadline, greeter_new_deadlock, greeter_new_fanout,
    greeter_new_front_desk, greeter_new_journal, greeter_new_ping_pong, greeter_new_ring,
    greeter_new_ticker,
};
use sans_effort_core::boundary::codec::Decode;
use sans_effort_host::record::{Log, replay};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (Some(path), Some(mode)) = (args.get(1), args.get(2)) else {
        eprintln!(
            "usage: replay LOG MODE   (MODE: greeter, fanout, ticker, ping-pong, front-desk, ring, journal, deadline, deadlock)"
        );
        return ExitCode::from(2);
    };
    // Plain `fn`s, which `replay` can call; the constructors are `extern "C"`.
    let root: fn() -> u64 = match mode.as_str() {
        "greeter" => || greeter_new(),
        "fanout" => || greeter_new_fanout(),
        "ticker" => || greeter_new_ticker(),
        "ping-pong" => || greeter_new_ping_pong(),
        "front-desk" => || greeter_new_front_desk(),
        "ring" => || greeter_new_ring(),
        "journal" => || greeter_new_journal(),
        "deadline" => || greeter_new_deadline(),
        "deadlock" => || greeter_new_deadlock(),
        other => {
            eprintln!("no such mode: {other}");
            return ExitCode::from(2);
        }
    };
    let log = match std::fs::read(path).map(|bytes| Log::from_bytes(&bytes)) {
        Ok(Ok(log)) => log,
        Ok(Err(e)) => {
            eprintln!("{path} is not a recording: {e}");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    match replay(&log, root) {
        Ok(()) => {
            println!(
                "replays exactly: {} calls on {} machines",
                log.events().len(),
                log.handles().len()
            );
            ExitCode::SUCCESS
        }
        Err(divergence) => {
            println!("diverges: {divergence}");
            ExitCode::FAILURE
        }
    }
}
