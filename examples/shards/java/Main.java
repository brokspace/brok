// A Brokspace shard in Java 17 or newer, with the standard library only.
//
// - on `file:changed`, publishes `todo-java:todo` for each TODO or FIXME line;
// - on `todo-java:ask` {query}, searches the index and publishes
//   `todo-java:answer` with the files found.
//
// The protocol is described in docs/shards-other-languages.md.
import java.io.IOException;
import java.net.UnixDomainSocketAddress;
import java.nio.ByteBuffer;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

public class Main {
    static final String NAME = System.getenv("BROK_SHARD_NAME");
    static final long MAX_FILE_BYTES = 256 * 1024;
    static SocketChannel channel;

    public static void main(String[] args) throws IOException {
        Path root = Path.of(System.getenv("BROK_WORKSPACE")).toRealPath();
        channel = SocketChannel.open(UnixDomainSocketAddress.of(System.getenv("BROK_ENDPOINT")));
        send(object("type", "hello", "protocol", 1, "name", NAME, "version", "0.1.0",
                "token", System.getenv("BROK_TOKEN")));
        Map<String, Object> welcome = receive();
        if (welcome == null || !"welcome".equals(welcome.get("type"))) {
            throw new IllegalStateException("the bus refused the connection: " + welcome);
        }
        send(object("type", "subscribe", "patterns", List.of("file:changed", NAME + ":ask")));
        log("ready");
        Set<Object> searches = new HashSet<>(); // ids of the searches waiting for a reply
        for (Map<String, Object> frame = receive(); frame != null; frame = receive()) {
            if ("error".equals(frame.get("type"))) {
                log("bus error: " + frame.get("message"));
                continue;
            }
            Map<String, Object> event = Json.object(frame.get("event"));
            String topic = (String) event.get("topic");
            Map<String, Object> payload = Json.object(event.get("payload"));
            if (topic.equals("system:shutdown")) {
                break;
            } else if (topic.equals("file:changed")) {
                for (Object file : (List<?>) payload.get("changed")) {
                    report(root, (String) file);
                }
            } else if (topic.equals(NAME + ":ask")) {
                searches.add(publish("index:search", object("query", payload.get("query"), "limit", 5)));
            } else if (searches.remove(event.get("correlation_id"))) {
                List<Object> paths = new ArrayList<>();
                for (Object hit : (List<?>) payload.get("hits")) {
                    paths.add(Json.object(hit).get("path"));
                }
                publish(NAME + ":answer", object("paths", paths));
            }
        }
    }

    /** Publishes each marker of a workspace file. */
    static void report(Path root, String file) throws IOException {
        List<String> lines;
        try {
            Path full = root.resolve(file).toRealPath();
            // Never read outside the workspace, whatever the event says.
            if (!full.startsWith(root) || !Files.isRegularFile(full) || Files.size(full) > MAX_FILE_BYTES) {
                return;
            }
            lines = new String(Files.readAllBytes(full), StandardCharsets.UTF_8).lines().toList();
        } catch (IOException deletedMeanwhile) {
            return;
        }
        for (int i = 0; i < lines.size(); i++) {
            String text = lines.get(i).strip();
            if (text.contains("TODO") || text.contains("FIXME")) {
                publish(NAME + ":todo", object("path", file, "line", i + 1, "text", text));
            }
        }
    }

    /** stdout and stderr both end up in .brokspace/logs/<name>.log. */
    static void log(String message) {
        System.err.println(message);
    }

    static String publish(String topic, Map<String, Object> payload) throws IOException {
        String id = UUID.randomUUID().toString();
        send(object("type", "publish", "event", object(
                "id", id, "topic", topic, "timestamp_ms", System.currentTimeMillis(), "payload", payload)));
        return id;
    }

    /** An ordered map from alternating keys and values. */
    static Map<String, Object> object(Object... pairs) {
        Map<String, Object> map = new LinkedHashMap<>();
        for (int i = 0; i < pairs.length; i += 2) {
            map.put((String) pairs[i], pairs[i + 1]);
        }
        return map;
    }

    /** Frames are JSON preceded by their length, four bytes big endian. */
    static void send(Map<String, Object> frame) throws IOException {
        byte[] data = Json.write(frame).getBytes(StandardCharsets.UTF_8);
        ByteBuffer out = ByteBuffer.allocate(4 + data.length).putInt(data.length).put(data).flip();
        while (out.hasRemaining()) {
            channel.write(out);
        }
    }

    /** The next frame, or null when the bus closed the connection. */
    static Map<String, Object> receive() throws IOException {
        ByteBuffer head = read(4);
        ByteBuffer body = head == null ? null : read(head.getInt());
        return body == null ? null : Json.object(Json.read(StandardCharsets.UTF_8.decode(body).toString()));
    }

    static ByteBuffer read(int length) throws IOException {
        ByteBuffer in = ByteBuffer.allocate(length);
        while (in.hasRemaining()) {
            if (channel.read(in) < 0) {
                return null;
            }
        }
        return in.flip();
    }
}
