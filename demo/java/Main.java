/// The greeter driven from Java over the C ABI, with Panama and no native glue.
///
/// Java, like Python, has no executor that can poll a Rust future, so it takes
/// the host's role: `start`, then `reply` by request id until `COMPLETE`. This
/// file is `ABI.md` as ~200 lines of Java: downcall handles for the exports, a
/// little-endian codec, the greeter's tag table, and the loop.
///
/// Where the Python host performs each effect before replying, this one is
/// shaped like a production host: every ask runs on its own virtual thread,
/// completions come back through a queue, and one driver thread makes every
/// call into the library (the ABI allows one caller per handle at a time).
/// Replies therefore arrive in whatever order the effects finish, and a
/// closed frame cancels the effect's thread. The transcript must still match
/// every other host's byte for byte.
///
/// With spawning, the driver thread is a small scheduler: it keeps every
/// machine by handle, starts each child it is told about (tag 6, or tag 7 for
/// a child pinned to the thread that starts it — this one), and when no
/// completion is waiting, resumes every IDLE machine once. Messages between
/// routines never pass through here; resuming is how a receiver finds one.
///
/// Compare `../js/main.mjs`, where the event loop is an executor and the same
/// routine runs natively.
///
///     cargo build -p greeter_cdylib
///     java --enable-native-access=ALL-UNNAMED demo/java/Main.java [--fanout | --ticker | --ping-pong | --front-desk]

import java.lang.foreign.*;
import java.lang.invoke.MethodHandle;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicLong;

public class Main {
    // ---- ABI.md, as code --------------------------------------------------

    static final byte ABI_VERSION = 0;
    static final int FRAME_TELL = 1, FRAME_ASK = 2, FRAME_CLOSED = 3;
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

    /** `abi_version / new / start / reply / resume / free / buf_free` over the loaded cdylib, prefix `greeter_`. */
    static final class Library {
        final Arena arena = Arena.ofConfined();
        final MethodHandle abiVersion, newGreeter, newFanout, newTicker, newPingPong, newFrontDesk, start, reply, resume, free, bufFree;

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
            start      = linker.downcallHandle(lib.find("greeter_start").get(), FunctionDescriptor.of(i32, u64, ptr, ptr));
            reply      = linker.downcallHandle(lib.find("greeter_reply").get(), FunctionDescriptor.of(i32, u64, ptr, u64, ptr, ptr));
            resume     = linker.downcallHandle(lib.find("greeter_resume").get(), FunctionDescriptor.of(i32, u64, ptr, ptr));
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
                default -> (long) newGreeter.invokeExact();
            };
        }

        /** Run to the first wait; the effect bytes out. Status via {@link #status}. */
        byte[] start(long handle) throws Throwable {
            try (Arena call = Arena.ofConfined()) {
                MemorySegment outPtr = call.allocate(ValueLayout.ADDRESS);
                MemorySegment outLen = call.allocate(ValueLayout.JAVA_LONG);
                int code = (int) start.invokeExact(handle, outPtr, outLen);
                return collect(code, outPtr, outLen);
            }
        }

        /** One reply record in; the effect bytes out. */
        byte[] reply(long handle, byte[] record) throws Throwable {
            try (Arena call = Arena.ofConfined()) {
                MemorySegment in = call.allocate(record.length);
                in.copyFrom(MemorySegment.ofArray(record));
                MemorySegment outPtr = call.allocate(ValueLayout.ADDRESS);
                MemorySegment outLen = call.allocate(ValueLayout.JAVA_LONG);
                int code = (int) reply.invokeExact(handle, in, (long) record.length, outPtr, outLen);
                return collect(code, outPtr, outLen);
            }
        }

        /** Poll again with nothing to deliver; the effect bytes out. Harmless when nothing changed. */
        byte[] resume(long handle) throws Throwable {
            try (Arena call = Arena.ofConfined()) {
                MemorySegment outPtr = call.allocate(ValueLayout.ADDRESS);
                MemorySegment outLen = call.allocate(ValueLayout.JAVA_LONG);
                int code = (int) resume.invokeExact(handle, outPtr, outLen);
                return collect(code, outPtr, outLen);
            }
        }

        int status;

        private byte[] collect(int code, MemorySegment outPtr, MemorySegment outLen) throws Throwable {
            if (code < 0) throw new RuntimeException(ERRORS.getOrDefault(code, "code " + code));
            status = code;
            long len = outLen.get(ValueLayout.JAVA_LONG, 0);
            MemorySegment ptr = outPtr.get(ValueLayout.ADDRESS, 0).reinterpret(len);
            byte[] data = ptr.toArray(ValueLayout.JAVA_BYTE);
            bufFree.invokeExact(ptr, len);
            return data;
        }

        void free(long handle) throws Throwable {
            int code = (int) free.invokeExact(handle);
            if (code < 0) throw new RuntimeException(ERRORS.getOrDefault(code, "code " + code));
        }
    }

    // ---- the greeter's tag table (the program's, not the ABI's) ------------

    /** One decoded effect. `id` is a request id, or the child's handle for `spawned`/`spawned_pinned`. */
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

    /** A finished effect: the request it answers and the reply record for it. */
    record Completion(Request request, byte[] record) {}

    /** Performs asks concurrently; the driver thread alone talks to the library. */
    static final class Host implements AutoCloseable {
        final Library lib;
        final Iterator<String> lines;
        final AtomicLong greeted = new AtomicLong();
        final ExecutorService pool = Executors.newVirtualThreadPerTaskExecutor();
        final BlockingQueue<Completion> done = new LinkedBlockingQueue<>();
        final Map<Request, Future<?>> inFlight = new HashMap<>();
        /** Every live machine, by handle, and the status of its last call. Insertion order is resume order. */
        final Map<Long, Integer> machines = new LinkedHashMap<>();

        Host(Library lib, List<String> script) { this.lib = lib; lines = script.iterator(); }

        /** Record what a call on `handle` returned, act on its effects, and free the machine if it completed. */
        void ran(long handle, byte[] data) throws Throwable {
            machines.put(handle, lib.status);
            dispatch(handle, decode(data));
            if (lib.status == COMPLETE) {
                machines.remove(handle);
                lib.free(handle);
            }
        }

        /** Tells run here, in order; each child starts here, on the driver thread; each ask starts on its own thread; closed ids are cancelled. */
        void dispatch(long handle, List<Effect> effects) throws Throwable {
            for (Effect e : effects) {
                switch (e.kind()) {
                    case "write_line" -> System.out.println(e.text());
                    case "spawned", "spawned_pinned" -> ran(e.id(), lib.start(e.id()));
                    case "closed" -> {
                        Future<?> work = inFlight.remove(new Request(handle, e.id()));
                        if (work != null) work.cancel(true);
                    }
                    default -> {
                        Request request = new Request(handle, e.id());
                        inFlight.put(request, pool.submit(() -> {
                            done.put(new Completion(request, perform(e)));
                            return null;
                        }));
                    }
                }
            }
        }

        /** Resume every idle machine once. `true` if any recorded something or stopped being idle. */
        boolean resumeIdle() throws Throwable {
            boolean progressed = false;
            for (long handle : List.copyOf(machines.keySet())) {
                Integer status = machines.get(handle);
                if (status == null || status != IDLE) continue;
                byte[] data = lib.resume(handle);
                progressed |= data.length > 0 || lib.status != IDLE;
                ran(handle, data);
            }
            return progressed;
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

        @Override
        public void close() { pool.close(); }
    }

    static void drive(Library lib, long root, List<String> script) throws Throwable {
        try (Host host = new Host(lib, script)) {
            host.ran(root, lib.start(root));

            while (true) {
                Completion c = host.done.poll();
                if (c == null) {
                    // No completion waiting: idle machines may have messages.
                    if (host.resumeIdle()) continue;
                    if (host.inFlight.isEmpty()) break;
                    c = host.done.take();
                }
                // Closed while in flight: the routine no longer wants it, and a reply would be STALE.
                if (host.inFlight.remove(c.request()) == null) continue;
                long handle = c.request().handle();
                host.ran(handle, lib.reply(handle, c.record()));
            }

            if (!host.machines.isEmpty())
                throw new IllegalStateException("stalled: machines " + host.machines.keySet() + " can never progress");
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

    public static void main(String[] args) throws Throwable {
        List<String> argv = List.of(args);
        String kind = List.of("fanout", "ticker", "ping-pong", "front-desk").stream()
            .filter(k -> argv.contains("--" + k)).findFirst().orElse("greeter");
        List<String> script = switch (kind) {
            case "fanout" -> List.of("bob", "carol");
            case "ticker", "ping-pong" -> List.of();
            case "front-desk" -> List.of("alice", "bob", "carol");
            default -> List.of("alice", "bob");
        };

        Library lib = new Library(findLibrary());
        drive(lib, lib.create(kind), script);
    }
}
