// Just enough JSON to keep this example free of dependencies. In your shard,
// use Jackson or Gson and bundle it into the jar.
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

final class Json {
    private final String text;
    private int at;

    private Json(String text) {
        this.text = text;
    }

    static Object read(String text) {
        return new Json(text).value();
    }

    @SuppressWarnings("unchecked")
    static Map<String, Object> object(Object value) {
        return value instanceof Map ? (Map<String, Object>) value : new LinkedHashMap<>();
    }

    static String write(Object value) {
        if (value instanceof Map<?, ?> map) {
            List<String> fields = new ArrayList<>();
            map.forEach((k, v) -> fields.add(write(k) + ":" + write(v)));
            return "{" + String.join(",", fields) + "}";
        }
        if (value instanceof List<?> list) {
            return "[" + String.join(",", list.stream().map(Json::write).toList()) + "]";
        }
        if (value instanceof String s) {
            StringBuilder out = new StringBuilder("\"");
            for (char c : s.toCharArray()) {
                if (c == '"' || c == '\\') {
                    out.append('\\').append(c);
                } else if (c < 0x20) {
                    out.append(String.format("\\u%04x", (int) c));
                } else {
                    out.append(c);
                }
            }
            return out.append('"').toString();
        }
        return String.valueOf(value); // numbers, booleans and null
    }

    private void skipSpace() {
        while (at < text.length() && Character.isWhitespace(text.charAt(at))) {
            at++;
        }
    }

    /** Consumes `c` if it comes next. */
    private boolean next(char c) {
        skipSpace();
        if (text.charAt(at) != c) {
            return false;
        }
        at++;
        return true;
    }

    private Object value() {
        if (next('{')) {
            Map<String, Object> map = new LinkedHashMap<>();
            while (!next('}')) {
                String key = (String) value();
                next(':');
                map.put(key, value());
                next(',');
            }
            return map;
        }
        if (next('[')) {
            List<Object> list = new ArrayList<>();
            while (!next(']')) {
                list.add(value());
                next(',');
            }
            return list;
        }
        if (next('"')) {
            return string();
        }
        int start = at;
        while (at < text.length() && ",}] \n\r\t".indexOf(text.charAt(at)) < 0) {
            at++;
        }
        String word = text.substring(start, at);
        return switch (word) {
            case "true" -> true;
            case "false" -> false;
            case "null" -> null;
            default -> Double.valueOf(word);
        };
    }

    private String string() {
        StringBuilder out = new StringBuilder();
        for (char c = text.charAt(at++); c != '"'; c = text.charAt(at++)) {
            if (c != '\\') {
                out.append(c);
                continue;
            }
            char escape = text.charAt(at++);
            switch (escape) {
                case 'n' -> out.append('\n');
                case 't' -> out.append('\t');
                case 'r' -> out.append('\r');
                case 'b' -> out.append('\b');
                case 'f' -> out.append('\f');
                case 'u' -> {
                    out.append((char) Integer.parseInt(text.substring(at, at + 4), 16));
                    at += 4;
                }
                default -> out.append(escape);
            }
        }
        return out.toString();
    }
}
