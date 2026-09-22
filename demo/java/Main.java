/// The greeter driven from Java over the C ABI, with Panama and no native glue.
///
/// Java, like Python, has no executor that can poll a Rust future, so it takes
/// the host's role: `start`, then `reply` by request id until `COMPLETE`. This
/// file is `ABI.md` as ~150 lines of Java: downcall handles for the five
/// exports, a little-endian codec, the greeter's tag table, and the loop.
/// Compare `../js/main.mjs`, where the event loop is an executor and the same
/// routine runs natively.
///
///     cargo build -p greeter_cdylib
///     java --enable-native-access=ALL-UNNAMED demo/java/Main.java [--fanout | --ticker]

import java.lang.foreign.*;
import java.lang.invoke.MethodHandle;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;

public class Main {
    // ---- ABI.md, as code --------------------------------------------------

    static final byte ABI_VERSION = 0;
    static final int AWAITING = 0, COMPLETE = 1, STALLED = 2;
    static final Map<Integer, String> ERRORS = Map.of(
        -1, "BUSY", -2, "FINISHED", -3, "WRONG_KIND", -4, "PANICKED", -5, "BAD_HANDLE", -6, "BAD_INPUT");

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

    /** Little-endian records; `str` is u32 length + UTF-8. */
    static final class Reader {
        final ByteBuffer buf;
        Reader(byte[] data) { buf = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN); }
        int u8() { return Byte.toUnsignedInt(buf.get()); }
        long u64() { return buf.getLong(); }
        String str() {
            byte[] b = new byte[buf.getInt()];
            buf.get(b);
            return new String(b, StandardCharsets.UTF_8);
        }
        boolean done() { return !buf.hasRemaining(); }
    }

    /** `abi_version / new / start / reply / free / buf_free` over the loaded cdylib, prefix `greeter_`. */
    static final class Library {
        final Arena arena = Arena.ofConfined();
        final MethodHandle abiVersion, newGreeter, newFanout, newTicker, start, reply, free, bufFree;

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
            start      = linker.downcallHandle(lib.find("greeter_start").get(), FunctionDescriptor.of(i32, u64, ptr, ptr));
            reply      = linker.downcallHandle(lib.find("greeter_reply").get(), FunctionDescriptor.of(i32, u64, ptr, u64, ptr, ptr));
            free       = linker.downcallHandle(lib.find("greeter_free").get(), FunctionDescriptor.of(i32, u64));
            bufFree    = linker.downcallHandle(lib.find("greeter_buf_free").get(), FunctionDescriptor.ofVoid(ptr, u64));

            byte v = (byte) abiVersion.invokeExact();
            if (v != ABI_VERSION) throw new IllegalStateException("ABI revision " + v + "; this host speaks " + ABI_VERSION);
        }

        long create(String kind) throws Throwable {
            return switch (kind) {
                case "fanout" -> (long) newFanout.invokeExact();
                case "ticker" -> (long) newTicker.invokeExact();
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

    record Effect(String kind, long id, String name, long millis, String text) {}

    static List<Effect> decode(byte[] data) {
        Reader r = new Reader(data);
        List<Effect> out = new ArrayList<>();
        while (!r.done()) {
            int tag = r.u8();
            out.add(switch (tag) {
                case 1 -> new Effect("count", r.u64(), null, 0, null);
                case 2 -> { String name = r.str(); yield new Effect("lookup", r.u64(), name, 0, null); }
                case 3 -> new Effect("read_line", r.u64(), null, 0, null);
                case 4 -> { long millis = r.u64(); yield new Effect("sleep", r.u64(), null, millis, null); }
                case 5 -> new Effect("write_line", 0, null, 0, r.str());
                default -> throw new IllegalStateException("unknown effect tag " + tag + ": this host knows 1..5");
            });
        }
        return out;
    }

    // ---- the host loop ----------------------------------------------------

    static final Map<String, String> GREETINGS = Map.of("alice", "Hello", "bob", "Hi", "carol", "Hey");

    static void drive(Library lib, long handle, List<String> script) throws Throwable {
        Iterator<String> lines = script.iterator();
        long greeted = 0;
        Deque<Effect> queue = new ArrayDeque<>(decode(lib.start(handle)));

        while (!queue.isEmpty()) {
            Effect e = queue.poll();
            byte[] record;
            switch (e.kind()) {
                case "write_line" -> { System.out.println(e.text()); continue; }
                case "read_line" -> record = replyStr(e.id(), lines.hasNext() ? lines.next() : "quit");
                case "lookup" -> record = replyStr(e.id(), GREETINGS.getOrDefault(e.name(), "Greetings"));
                case "sleep" -> { Thread.sleep(e.millis()); record = replyUnit(e.id()); }
                case "count" -> record = replyU64(e.id(), ++greeted);
                default -> throw new IllegalStateException(e.kind());
            }
            queue.addAll(decode(lib.reply(handle, record)));
        }

        if (lib.status != COMPLETE) throw new IllegalStateException("routine ended with status " + lib.status);
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
        String kind = argv.contains("--fanout") ? "fanout" : argv.contains("--ticker") ? "ticker" : "greeter";
        List<String> script = switch (kind) {
            case "fanout" -> List.of("bob", "carol");
            case "ticker" -> List.of();
            default -> List.of("alice", "bob");
        };

        Library lib = new Library(findLibrary());
        long handle = lib.create(kind);
        try {
            drive(lib, handle, script);
        } finally {
            lib.free(handle);
        }
    }
}
