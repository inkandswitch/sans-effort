//! A routine that uses the rest of the standard library: the environment,
//! files, the clock, and randomness.

use alloc::{format, string::String, vec::Vec};
use core::{fmt::Write, ops::ControlFlow};
use sans_effort::{
    console::WriteLine,
    env::Var,
    fs::{FsError, ReadFile, WriteFile},
    random::Random,
    step::Step,
    time::Now,
};

/// The variable naming the journal file.
pub const JOURNAL_VAR: &str = "JOURNAL";

/// The file used when [`JOURNAL_VAR`] is unset.
pub const DEFAULT_PATH: &str = "journal.txt";

/// Appends `n` entries to a journal file, one per step: reads the file (a
/// missing one is empty), stamps a new line with the time and a random id,
/// writes the file back, and says what it wrote.
///
/// Every value it depends on comes from its context, so a host decides all
/// of them — which is how the demo's hosts agree byte for byte: each answers
/// `JOURNAL` with `notes.txt`, keeps files in memory, starts its clock at
/// 1 700 000 000 000 ms and moves it 1 000 ms per reading, and counts its
/// random bytes up from `00`.
#[derive(Debug)]
pub struct Journal<C> {
    ctx: C,
    remaining: u32,
}

impl<C> Journal<C> {
    /// Append `n` entries through `ctx`.
    pub const fn new(ctx: C, n: u32) -> Self {
        Self { ctx, remaining: n }
    }
}

impl<C: Var + ReadFile + WriteFile + Now + Random + WriteLine> Step for Journal<C> {
    async fn step(&mut self) -> ControlFlow<()> {
        if self.remaining == 0 {
            return ControlFlow::Break(());
        }
        self.remaining -= 1;

        let path = self
            .ctx
            .var(JOURNAL_VAR.into())
            .await
            .unwrap_or_else(|| DEFAULT_PATH.into());

        let mut text = match self.ctx.read_file(path.clone()).await {
            Ok(bytes) => bytes,
            Err(FsError::NotFound) => Vec::new(),
            Err(e) => {
                self.ctx.write_line(format!("cannot read {path}: {e}"));
                return ControlFlow::Break(());
            }
        };

        let at = self.ctx.now().await.since_epoch().as_millis();
        let id = hex(&self.ctx.random_bytes(4).await);
        let entry = format!("{at} {id}");
        text.extend_from_slice(entry.as_bytes());
        text.push(b'\n');
        let count = text
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .count();

        if let Err(e) = self.ctx.write_file(path.clone(), text).await {
            self.ctx.write_line(format!("cannot write {path}: {e}"));
            return ControlFlow::Break(());
        }
        self.ctx
            .write_line(format!("{path}: entry {count}, {entry}"));
        ControlFlow::Continue(())
    }
}

/// Lowercase hex, two digits a byte.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _infallible = write!(out, "{b:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use alloc::{collections::BTreeMap, sync::Arc, vec};
    use core::{
        future::{Future, ready},
        time::Duration,
    };
    use sans_effort::{testing::run_now, time::UnixTime};
    use std::sync::Mutex;

    /// The demo hosts' fixtures, in Rust: a fixed variable, files in memory,
    /// a clock that moves a second per reading, counting random bytes, and a
    /// log of lines written.
    #[derive(Clone, Default)]
    struct Desk(Arc<State>);

    #[derive(Default)]
    struct State {
        var: Option<String>,
        files: Mutex<BTreeMap<String, Vec<u8>>>,
        readings: Mutex<u64>,
        next_byte: Mutex<u8>,
        written: Mutex<Vec<String>>,
        refuse_writes: bool,
    }

    impl Var for Desk {
        fn var(&self, _: String) -> impl Future<Output = Option<String>> + Send {
            ready(self.0.var.clone())
        }
    }

    impl ReadFile for Desk {
        fn read_file(&self, path: String) -> impl Future<Output = Result<Vec<u8>, FsError>> + Send {
            let files = self.0.files.lock().expect("unpoisoned");
            ready(files.get(&path).cloned().ok_or(FsError::NotFound))
        }
    }

    impl WriteFile for Desk {
        fn write_file(
            &self,
            path: String,
            bytes: Vec<u8>,
        ) -> impl Future<Output = Result<(), FsError>> + Send {
            if self.0.refuse_writes {
                return ready(Err(FsError::PermissionDenied));
            }
            self.0.files.lock().expect("unpoisoned").insert(path, bytes);
            ready(Ok(()))
        }
    }

    impl Now for Desk {
        fn now(&self) -> impl Future<Output = UnixTime> + Send {
            let mut readings = self.0.readings.lock().expect("unpoisoned");
            let millis = 1_700_000_000_000 + 1_000 * *readings;
            *readings += 1;
            ready(UnixTime::from_since_epoch(Duration::from_millis(millis)))
        }
    }

    impl Random for Desk {
        fn random_bytes(&self, len: u32) -> impl Future<Output = Vec<u8>> + Send {
            let mut next = self.0.next_byte.lock().expect("unpoisoned");
            let bytes = (0..len)
                .map(|_| {
                    let b = *next;
                    *next = next.wrapping_add(1);
                    b
                })
                .collect();
            ready(bytes)
        }
    }

    impl WriteLine for Desk {
        fn write_line(&self, line: String) {
            self.0.written.lock().expect("unpoisoned").push(line);
        }
    }

    /// For any number of entries and any variable, set or not: the named
    /// file (or the default) ends with one line per entry, each stamped a
    /// second after the last with the next four random bytes, and each entry
    /// is announced with its number.
    #[test]
    fn appends_stamped_entries_to_the_named_file() {
        bolero::check!()
            .with_type::<(u8, Option<String>)>()
            .for_each(|(n, var)| {
                let desk = Desk(Arc::new(State {
                    var: var.clone(),
                    ..State::default()
                }));
                run_now(Journal::new(desk.clone(), (*n).into()).run());

                let path = var.as_deref().unwrap_or(DEFAULT_PATH);
                let entries: Vec<String> = (0..u64::from(*n))
                    .map(|k| {
                        let byte = |b: u64| u8::try_from((4 * k + b) % 256).expect("a byte");
                        let id = u32::from_be_bytes([byte(0), byte(1), byte(2), byte(3)]);
                        format!("{} {id:08x}", 1_700_000_000_000 + 1_000 * k)
                    })
                    .collect();
                let announced: Vec<String> = entries
                    .iter()
                    .enumerate()
                    .map(|(k, entry)| format!("{path}: entry {}, {entry}", k + 1))
                    .collect();
                let file = format!("{}\n", entries.join("\n"));

                assert_eq!(*desk.0.written.lock().expect("unpoisoned"), announced);
                assert_eq!(
                    desk.0.files.lock().expect("unpoisoned").get(path).cloned(),
                    (*n > 0).then(|| file.into_bytes())
                );
            });
    }

    #[test]
    fn a_refused_write_is_reported_and_ends_the_journal() {
        let desk = Desk(Arc::new(State {
            refuse_writes: true,
            ..State::default()
        }));
        run_now(Journal::new(desk.clone(), 3).run());
        assert_eq!(
            *desk.0.written.lock().expect("unpoisoned"),
            vec!["cannot write journal.txt: permission denied"]
        );
    }
}
