/// The greeter driven from Java over the C ABI, with Panama and no native glue.
///
/// Java, like Python, has no executor that can poll a Rust future, so it takes
/// the host's role: `start`, then `reply` by request id until `COMPLETE`. This
/// file is `ABI.md` as ~200 lines of Java: downcall handles for the exports, a
/// little-endian codec, the greeter's tag table, and the loop.
///
/// Where the Python host performs each effect before replying, this one is
/// shaped like a production host, and it is parallel:
///
/// - A pool of driver threads — platform threads, one per core — makes the
///   calls into the library, so different machines are polled at the same
///   time. Each machine takes one event at a time from its own inbox (start,
///   reply, resume), so no two threads ever call one handle and `BUSY` cannot
///   happen.
/// - A machine the library can move (tag 6) is polled on whichever worker is
///   next, so its calls migrate between threads. A pinned one (tag 7) is
///   polled only on the worker that started it, so `WRONG_THREAD` cannot
///   happen either.
/// - Every ask runs on its own virtual thread; its reply becomes an event in
///   the machine's inbox, in whatever order the effects finish, and a closed
///   frame cancels it.
/// - A message between routines never passes through here. The call whose
///   routine sent it reports whom that woke, as a `woke` frame, and the host
///   resumes that machine. When everything has drained — no events queued, no
///   asks in flight — it asks `wakes` for anything woken outside a call; if
///   that is empty too, the run is over, or stalled.
///
/// The transcript must still match every other host's byte for byte. It can,
/// because in each demo one machine does the writing: a host prints a
/// machine's writes after its call returns, so writes from two machines polled
/// at once would interleave by timing.
///
/// Compare `../js/main.mjs`, where the event loop is an executor and the same
/// routine runs natively.
///
///     cargo build -p greeter_cdylib
///     java --enable-native-access=ALL-UNNAMED demo/java/Main.java [--fanout | --ticker | --ping-pong | --front-desk | --ring] [--trace]
///
/// `--trace` logs every call to stderr — thread, call, handle, status — to
/// show which worker polled what.

import java.lang.foreign.*;
import java.lang.invoke.MethodHandle;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;

public class Main {
    // ---- ABI.md, as code --------------------------------------------------

    static final byte ABI_VERSION = 0;
    static final int FRAME_TELL = 1, FRAME_ASK = 2, FRAME_CLOSED = 3, FRAME_WOKE = 4;
    static final int AWAITING = 0, COMPLETE = 1, IDLE = 2;
    static final Map<Integer, String> ERRORS = Map.of(
        -1, "BUSY", -2, "FINISHED", -3, "WRONG_KIND", -4, "PANICKED", -5, "BAD_HANDLE", -6, "BAD_INPUT",
        -7, "MALFORMED", -8, "STALE", -9, "WRONG_THREAD");

    /** Reply records: kind, then request id, then payload. */
    static byte[] replyStr(long id, String s) {
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        return ByteBuffer.allocate(1 + 8 + 4 + b.length).order(ByteOrder.LITTLE_ENDIAN)
            .put((byte) 1).putLong(id).putInt(b.length).put(b).array();
    }

    static byte[] replyU64(long id, long n) {
        return ByteBuffer.allocate(1 + 8 + 8).order(ByteOrder.LITTLE_ENDIAN)
            .put((byte) 2).putLong(id).putLong(n).array();
    }

    static byte[] replyUnit(long id) {
        return ByteBuffer.allocate(1 + 8).order(ByteOrder.LITTLE_ENDIAN)
            .put((byte) 3).putLong(id).array();
    }

    static byte[] replyBytes(long id, byte[] b) {
        return ByteBuffer.allocate(1 + 8 + 4 + b.length).order(ByteOrder.LITTLE_ENDIAN)
            .put((byte) 4).putLong(id).putInt(b.length).put(b).array();
    }

    /** Little-endian records; `str` is u32 length + UTF-8. */
    static final class Reader {
        final ByteBuffer buf;
        Reader(byte[] data) { buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN); }
        int u8() { return Byte.toUnsignedInt(buf.get()); }
        long u64() { return buf.getLong(); }
        byte[] bytes() {
            byte[] b = new byte[buf.getInt()];
            buf.get(b);
            return b;
        }
        String str() { return new String(bytes(), StandardCharsets.UTF_8); }
        boolean done() { return !buf.hasRemaining(); }
    }

    /** `abi_version / new / start / reply / resume / wakes / free / buf_free` over the loaded cdylib, prefix `greeter_`. */
    static final class Library {
        /** Shared: the symbols are called from every driver thread. */
        final Arena arena = Arena.ofShared();
        final MethodHandle abiVersion, newGreeter, newFanout, newTicker, newPingPong, newFrontDesk, newRing, start, reply, resume, wakes, free, bufFree;

        Library(Path path) throws Throwable {
            Linker linker = Linker.nativeLinker();
            SymbolLookup lib = SymbolLookup.libraryLookup(path, arena);
            var u64 = ValueLayout.JAVA_LONG;
            var i32 = ValueLayout.JAVA_INT;
            var u8  = ValueLayout.JAVA_BYTE;
            var ptr = ValueLayout.ADDRESS;
            abiVersion = linker.downcallHandle(lib.find("greeter_abi_version").get(), FunctionDescriptor.of(u8));
            newGreeter = linker.downcallHandle(lib.find("greeter_new").get(), FunctionDescriptor.of(u64));
            newFanout  = linker.downcallHandle(lib.find("greeter_new_fanout").get(), FunctionDescriptor.of(u64));
            newTicker  = linker.downcallHandle(lib.find("greeter_new_ticker").get(), FunctionDescriptor.of(u64));
            newPingPong  = linker.downcallHandle(lib.find("greeter_new_ping_pong").get(), FunctionDescriptor.of(u64));
            newFrontDesk = linker.downcallHandle(lib.find("greeter_new_front_desk").get(), FunctionDescriptor.of(u64));
            newRing      = linker.downcallHandle(lib.find("greeter_new_ring").get(), FunctionDescriptor.of(u64));
            start      = linker.downcallHandle(lib.find("greeter_start").get(), FunctionDescriptor.of(i32, u64, ptr, ptr));
            reply      = linker.downcallHandle(lib.find("greeter_reply").get(), FunctionDescriptor.of(i32, u64, ptr, u64, ptr, ptr));
            resume     = linker.downcallHandle(lib.find("greeter_resume").get(), FunctionDescriptor.of(i32, u64, ptr, ptr));
            wakes      = linker.downcallHandle(lib.find("greeter_wakes").get(), FunctionDescriptor.of(i32, ptr, ptr));
            free       = linker.downcallHandle(lib.find("greeter_free").get(), FunctionDescriptor.of(i32, u64));
            bufFree    = linker.downcallHandle(lib.find("greeter_buf_free").get(), FunctionDescriptor.ofVoid(ptr, u64));

            byte v = (byte) abiVersion.invokeExact();
            if (v != ABI_VERSION) throw new IllegalStateException("ABI revision " + v + "; this host speaks " + ABI_VERSION);
        }

        long create(String kind) throws Throwable {
            return switch (kind) {
                case "fanout" -> (long) newFanout.invokeExact();
                case "ticker" -> (long) newTicker.invokeExact();
                case "ping-pong" -> (long) newPingPong.invokeExact();
                case "front-desk" -> (long) newFrontDesk.invokeExact();
                case "ring" -> (long) newRing.invokeExact();
                default -> (long) newGreeter.invokeExact();
            };
        }

        /** Run to the first wait. */
        Outcome start(long handle) throws Throwable {
            try (Arena call = Arena.ofConfined()) {
                MemorySegment outPtr = call.allocate(ValueLayout.ADDRESS);
                MemorySegment outLen = call.allocate(ValueLayout.JAVA_LONG);
                int code = (int) start.invokeExact(handle, outPtr, outLen);
                return collect(code, outPtr, outLen);
            }
        }

        /** One reply record in; the next batch out. */
        Outcome reply(long handle, byte[] record) throws Throwable {
            try (Arena call = Arena.ofConfined()) {
                MemorySegment in = call.allocate(record.length);
                in.copyFrom(MemorySegment.ofArray(record));
                MemorySegment outPtr = call.allocate(ValueLayout.ADDRESS);
                MemorySegment outLen = call.allocate(ValueLayout.JAVA_LONG);
                int code = (int) reply.invokeExact(handle, in, (long) record.length, outPtr, outLen);
                return collect(code, outPtr, outLen);
            }
        }

        /** Poll again with nothing to deliver. Harmless when nothing changed. */
        Outcome resume(long handle) throws Throwable {
            try (Arena call = Arena.ofConfined()) {
                MemorySegment outPtr = call.allocate(ValueLayout.ADDRESS);
                MemorySegment outLen = call.allocate(ValueLayout.JAVA_LONG);
                int code = (int) resume.invokeExact(handle, outPtr, outLen);
                return collect(code, outPtr, outLen);
            }
        }

        /** The machines woken outside any call, as `woke` frames. */
        byte[] wakes() throws Throwable {
            try (Arena call = Arena.ofConfined()) {
                MemorySegment outPtr = call.allocate(ValueLayout.ADDRESS);
                MemorySegment outLen = call.allocate(ValueLayout.JAVA_LONG);
                int code = (int) wakes.invokeExact(outPtr, outLen);
                return collect(code, outPtr, outLen).data();
            }
        }

        private Outcome collect(int code, MemorySegment outPtr, MemorySegment outLen) throws Throwable {
            if (code < 0) throw new RuntimeException(ERRORS.getOrDefault(code, "code " + code));
            long len = outLen.get(ValueLayout.JAVA_LONG, 0);
            MemorySegment ptr = outPtr.get(ValueLayout.ADDRESS, 0).reinterpret(len);
            byte[] data = ptr.toArray(ValueLayout.JAVA_BYTE);
            bufFree.invokeExact(ptr, len);
            return new Outcome(code, data);
        }

        void free(long handle) throws Throwable {
            int code = (int) free.invokeExact(handle);
            if (code < 0) throw new RuntimeException(ERRORS.getOrDefault(code, "code " + code));
        }
    }

    /** What a call returned: its status code and the batch of effect bytes. */
    record Outcome(int status, byte[] data) {}

    // ---- the greeter's tag table (the program's, not the ABI's) ------------

    /** One decoded effect. `id` is a request id, or a machine's handle for `spawned`/`spawned_pinned`/`woke`. */
    record Effect(String kind, long id, String name, long millis, String text) {}

    /**
     * `read_line` is fallible, so it is answered with bytes: an encoded
     * `Result<String, ReadLineError>` — `00 · str` for a line, `01 · 00`
     * once the input is closed.
     */
    static byte[] readLineReply(long id, String line) {
        if (line == null) return replyBytes(id, new byte[] {1, 0});
        byte[] b = line.getBytes(StandardCharsets.UTF_8);
        return replyBytes(id, ByteBuffer.allocate(1 + 4 + b.length).order(ByteOrder.LITTLE_ENDIAN)
            .put((byte) 0).putInt(b.length).put(b).array());
    }

    /** One effect frame: its kind (tell, ask, or reserved) and the view's bytes. */
    record Frame(int kind, byte[] payload) {}

    /** Split a batch into frames. Knows nothing of any routine's tags. */
    static List<Frame> frames(byte[] data) {
        Reader r = new Reader(data);
        List<Frame> out = new ArrayList<>();
        while (!r.done()) out.add(new Frame(r.u8(), r.bytes()));
        return out;
    }

    static List<Effect> decode(byte[] data) {
        List<Effect> out = new ArrayList<>();
        List<Frame> fs = frames(data);
        for (int n = 0; n < fs.size(); n++) {
            Frame f = fs.get(n);
            if (f.kind() == FRAME_CLOSED) {
                out.add(new Effect("closed", new Reader(f.payload()).u64(), null, 0, null));
                continue;
            }
            if (f.kind() == FRAME_WOKE) {
                out.add(new Effect("woke", new Reader(f.payload()).u64(), null, 0, null));
                continue;
            }
            if (f.kind() != FRAME_TELL && f.kind() != FRAME_ASK) continue; // reserved: safe to skip
            Reader r = new Reader(f.payload());
            int tag = r.u8();
            Effect e = switch (tag) {
                case 1 -> new Effect("count", r.u64(), null, 0, null);
                case 2 -> { String name = r.str(); yield new Effect("lookup", r.u64(), name, 0, null); }
                case 3 -> new Effect("read_line", r.u64(), null, 0, null);
                case 4 -> { long millis = r.u64(); yield new Effect("sleep", r.u64(), null, millis, null); }
                case 5 -> new Effect("write_line", 0, null, 0, r.str());
                case 6 -> new Effect("spawned", r.u64(), null, 0, null);
                case 7 -> new Effect("spawned_pinned", r.u64(), null, 0, null);
                default -> null;
            };
            if (e == null) {
                if (f.kind() == FRAME_TELL) continue; // a tell this host doesn't know: skip it
                throw new IllegalStateException("frame " + n + " asks with unknown tag " + tag + ": this host cannot answer it; it knows 1..7");
            }
            if (!r.done())
                throw new IllegalStateException("frame " + n + " (" + e.kind() + ") has bytes this host did not expect: its tag table disagrees with the routine's");
            out.add(e);
        }
        return out;
    }

    // ---- the host loop ----------------------------------------------------

    static final Map<String, String> GREETINGS = Map.of("alice", "Hello", "bob", "Hi", "carol", "Hey");

    /** A request in flight: request ids are per machine, so the handle is part of its name. */
    record Request(long handle, long id) {}

    /** What a machine's inbox holds: something to call the library with. */
    sealed interface Event permits Start, Reply, Resume {}
    record Start() implements Event {}
    record Reply(long id, byte[] record) implements Event {}
    record Resume() implements Event {}

    /**
     * Drives every machine a root spawns, in parallel. Workers make the calls;
     * virtual threads perform the asks; the coordinating thread only waits for
     * quiet and asks `wakes`.
     */
    static final class Scheduler implements AutoCloseable {
        final Library lib;
        final Iterator<String> lines;
        final AtomicLong greeted = new AtomicLong();
        final ExecutorService[] workers;
        final AtomicInteger nextWorker = new AtomicInteger();
        final ExecutorService effects = Executors.newVirtualThreadPerTaskExecutor();
        final Map<Long, Machine> machines = new ConcurrentHashMap<>();
        final Map<Request, InFlight> inFlight = new ConcurrentHashMap<>();
        final AtomicReference<Throwable> failure = new AtomicReference<>();

        /** Events queued or running, plus asks in flight: zero means quiet. */
        int pending;

        /** One machine: its handle, where it may run, and its inbox. */
        final class Machine {
            final long handle;
            final boolean pinned;
            /** The worker a pinned machine was started on; -1 until then, and always for a migrating one. */
            int worker = -1;
            volatile int status = AWAITING;
            final ArrayDeque<Event> inbox = new ArrayDeque<>();
            boolean running;

            Machine(long handle, boolean pinned) { this.handle = handle; this.pinned = pinned; }
        }

        /** An ask being performed (`task` is null for the moment before it is submitted), settled exactly once: by its task, or by a cancel before it ran. */
        record InFlight(Future<?> task, AtomicBoolean settled) {}

        Scheduler(Library lib, List<String> script, int threads) {
            this.lib = lib;
            lines = script.iterator();
            workers = new ExecutorService[threads];
            for (int i = 0; i < threads; i++) workers[i] = Executors.newSingleThreadExecutor(Thread.ofPlatform().name("driver-" + i).factory());
        }

        void busy() { synchronized (this) { pending++; } }

        void settle() {
            synchronized (this) {
                if (--pending == 0) notifyAll();
            }
        }

        /** Wait until no event is queued or running and no ask is in flight. */
        void awaitQuiet() throws Throwable {
            synchronized (this) {
                while (pending > 0 && failure.get() == null) wait();
            }
            if (failure.get() != null) throw failure.get();
        }

        /** Queue `event` for `m`, and schedule its inbox if it is not already running. */
        void post(Machine m, Event event) {
            busy();
            boolean schedule;
            synchronized (m) {
                m.inbox.add(event);
                schedule = !m.running;
                m.running = true;
            }
            if (schedule) worker(m).execute(() -> drain(m));
        }

        /** A pinned machine's own worker (chosen when it starts); any worker for a migrating one. */
        ExecutorService worker(Machine m) {
            synchronized (m) {
                if (!m.pinned) return workers[Math.floorMod(nextWorker.getAndIncrement(), workers.length)];
                if (m.worker < 0) m.worker = Math.floorMod(nextWorker.getAndIncrement(), workers.length);
                return workers[m.worker];
            }
        }

        /**
         * Handle this machine's events one at a time, on this worker, until its
         * inbox is empty. A migrating machine's next batch may run elsewhere.
         */
        void drain(Machine m) {
            while (true) {
                Event event;
                synchronized (m) {
                    event = m.inbox.poll();
                    if (event == null) { m.running = false; return; }
                }
                try {
                    handle(m, event);
                } catch (Throwable t) {
                    failure.compareAndSet(null, t);
                    synchronized (this) { notifyAll(); }
                } finally {
                    settle();
                }
            }
        }

        void handle(Machine m, Event event) throws Throwable {
            if (!machines.containsKey(m.handle)) return; // completed and freed meanwhile
            Outcome out = switch (event) {
                case Start s -> lib.start(m.handle);
                case Resume r -> lib.resume(m.handle); // harmless if it has moved on since
                case Reply r -> {
                    // Closed while in flight: the routine no longer wants it, and a reply would be STALE.
                    if (inFlight.remove(new Request(m.handle, r.id())) == null) yield null;
                    yield lib.reply(m.handle, r.record());
                }
            };
            if (out == null) return;
            if (TRACE) System.err.printf("[%s] %s(%d) -> %d%n", Thread.currentThread().getName(),
                event.getClass().getSimpleName().toLowerCase(), m.handle, out.status());
            m.status = out.status();
            dispatch(m, decode(out.data()));
            if (out.status() == COMPLETE) {
                machines.remove(m.handle);
                lib.free(m.handle); // on this worker: a pinned machine's own thread
            }
        }

        /** Tells run here, in order; children are registered and started; asks go to virtual threads; closed ids are cancelled. */
        void dispatch(Machine m, List<Effect> effects) {
            for (Effect e : effects) {
                switch (e.kind()) {
                    case "write_line" -> System.out.println(e.text());
                    case "woke" -> {
                        Machine woken = machines.get(e.id());
                        if (woken != null) post(woken, new Resume());
                    }
                    case "spawned", "spawned_pinned" -> {
                        Machine child = new Machine(e.id(), e.kind().equals("spawned_pinned"));
                        machines.put(child.handle, child);
                        post(child, new Start());
                    }
                    case "closed" -> {
                        InFlight work = inFlight.remove(new Request(m.handle, e.id()));
                        if (work != null) {
                            if (work.task() != null) work.task().cancel(true);
                            if (work.settled().compareAndSet(false, true)) settle();
                        }
                    }
                    default -> ask(m, e);
                }
            }
        }

        void ask(Machine m, Effect e) {
            busy();
            AtomicBoolean settled = new AtomicBoolean();
            Request request = new Request(m.handle, e.id());
            // Registered before its task can finish, so the reply finds it; the
            // task itself is filled in just after.
            inFlight.put(request, new InFlight(null, settled));
            Future<?> running = effects.submit(() -> {
                try {
                    post(m, new Reply(e.id(), perform(e)));
                } catch (InterruptedException cancelled) {
                    // closed: nothing to reply
                } catch (Throwable t) {
                    failure.compareAndSet(null, t);
                } finally {
                    if (settled.compareAndSet(false, true)) settle();
                }
                return null;
            });
            inFlight.computeIfPresent(request, (k, v) -> new InFlight(running, settled));
        }

        byte[] perform(Effect e) throws InterruptedException {
            return switch (e.kind()) {
                case "read_line" -> {
                    synchronized (lines) { yield readLineReply(e.id(), lines.hasNext() ? lines.next() : null); }
                }
                case "lookup" -> replyStr(e.id(), GREETINGS.getOrDefault(e.name(), "Greetings"));
                case "sleep" -> { Thread.sleep(e.millis()); yield replyUnit(e.id()); }
                case "count" -> replyU64(e.id(), greeted.incrementAndGet());
                default -> throw new IllegalStateException(e.kind());
            };
        }

        /** Run `root` and everything it spawns until nothing can progress. */
        void run(long root) throws Throwable {
            Machine first = new Machine(root, false);
            machines.put(root, first);
            post(first, new Start());

            while (true) {
                awaitQuiet();
                // Quiet: anything woken outside a call? If not, nothing can happen again.
                List<Effect> woken = decode(lib.wakes());
                if (woken.isEmpty()) break;
                dispatch(first, woken);
            }

            if (!machines.isEmpty())
                throw new IllegalStateException("stalled: machines " + machines.keySet() + " can never progress");
        }

        @Override
        public void close() {
            effects.close();
            for (ExecutorService w : workers) w.close();
        }
    }

    static void drive(Library lib, long root, List<String> script) throws Throwable {
        int threads = Math.max(2, Runtime.getRuntime().availableProcessors());
        try (Scheduler scheduler = new Scheduler(lib, script, threads)) {
            scheduler.run(root);
        }
    }

    static Path findLibrary() {
        Path root = Paths.get("").toAbsolutePath();
        for (String profile : List.of("debug", "release"))
            for (String name : List.of("libgreeter_cdylib.so", "libgreeter_cdylib.dylib", "greeter_cdylib.dll")) {
                Path p = root.resolve("target").resolve(profile).resolve(name);
                if (Files.exists(p)) return p;
            }
        throw new IllegalStateException("build it first: cargo build -p greeter_cdylib");
    }

    static boolean TRACE;

    public static void main(String[] args) throws Throwable {
        List<String> argv = List.of(args);
        TRACE = argv.contains("--trace");
        String kind = List.of("fanout", "ticker", "ping-pong", "front-desk", "ring").stream()
            .filter(k -> argv.contains("--" + k)).findFirst().orElse("greeter");
        List<String> script = switch (kind) {
            case "fanout" -> List.of("bob", "carol");
            case "ticker", "ping-pong", "ring" -> List.of();
            case "front-desk" -> List.of("alice", "bob", "carol");
            default -> List.of("alice", "bob");
        };

        Library lib = new Library(findLibrary());
        long began = System.nanoTime();
        drive(lib, lib.create(kind), script);
        if (kind.equals("ring")) {
            int hops = 16 * 250;
            double ms = (System.nanoTime() - began) / 1e6;
            System.err.printf("%d hops in %.1f ms: %.2f µs per hop%n", hops, ms, ms * 1e3 / hops);
        }
    }
}
