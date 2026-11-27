// A Brokspace shard in TypeScript, with Node's standard library only.
//
// - on `file:changed`, publishes `todo-typescript:todo` for each TODO or FIXME line;
// - on `todo-typescript:ask` {query}, searches the index and publishes
//   `todo-typescript:answer` with the files found.
//
// The installed file runs from inside the workspace, whose package.json decides
// whether Node reads it as CommonJS or as a module. `process.getBuiltinModule`
// works in both, so there is no `import` or `require` here.
const net = process.getBuiltinModule('node:net');
const fs = process.getBuiltinModule('node:fs');
const path = process.getBuiltinModule('node:path');
const { randomUUID } = process.getBuiltinModule('node:crypto');

interface BusEvent {
    id: string;
    topic: string;
    timestamp_ms: number;
    payload: any;
    correlation_id?: string;
}
type ServerFrame =
    | { type: 'welcome' }
    | { type: 'event'; event: BusEvent }
    | { type: 'error'; message: string };

const NAME: string = process.env.BROK_SHARD_NAME!;
const ROOT: string = fs.realpathSync(process.env.BROK_WORKSPACE!);
const MAX_FILE_BYTES = 256 * 1024;
const socket = net.createConnection(process.env.BROK_ENDPOINT!);
/** Ids of the searches waiting for a reply. */
const searches = new Set<string>();

// stdout and stderr both end up in .brokspace/logs/<name>.log.
const log = (message: string): void => console.error(message);

/** Frames are JSON preceded by their length, four bytes big endian. */
function send(frame: object): void {
    const data = Buffer.from(JSON.stringify(frame));
    const head = Buffer.alloc(4);
    head.writeUInt32BE(data.length);
    socket.write(Buffer.concat([head, data]));
}

function publish(topic: string, payload: unknown): string {
    const event: BusEvent = { id: randomUUID(), topic, timestamp_ms: Date.now(), payload };
    send({ type: 'publish', event });
    return event.id;
}

/** Each marker in a workspace file. */
function todos(file: string): { line: number; text: string }[] {
    let full: string;
    try {
        full = fs.realpathSync(path.join(ROOT, file));
        const stat = fs.statSync(full);
        // Never read outside the workspace, whatever the event says.
        if (!full.startsWith(ROOT + path.sep) || !stat.isFile() || stat.size > MAX_FILE_BYTES) return [];
    } catch {
        return []; // deleted meanwhile
    }
    return fs
        .readFileSync(full, 'utf8')
        .split('\n')
        .map((text, index) => ({ line: index + 1, text: text.trim() }))
        .filter(({ text }) => text.includes('TODO') || text.includes('FIXME'));
}

function handle(frame: ServerFrame): void {
    if (frame.type === 'welcome') {
        send({ type: 'subscribe', patterns: ['file:changed', `${NAME}:ask`] });
        log('ready');
        return;
    }
    if (frame.type === 'error') {
        log(`bus error: ${frame.message}`);
        return;
    }
    const { topic, payload, correlation_id } = frame.event;
    if (topic === 'system:shutdown') {
        socket.end(() => process.exit(0));
    } else if (topic === 'file:changed') {
        for (const file of payload.changed ?? []) {
            for (const todo of todos(file)) publish(`${NAME}:todo`, { path: file, ...todo });
        }
    } else if (topic === `${NAME}:ask`) {
        searches.add(publish('index:search', { query: payload.query, limit: 5 }));
    } else if (correlation_id && searches.delete(correlation_id)) {
        publish(`${NAME}:answer`, { paths: payload.hits.map((hit: { path: string }) => hit.path) });
    }
}

let buffered = Buffer.alloc(0);
socket.on('connect', () =>
    send({ type: 'hello', protocol: 1, name: NAME, version: '0.1.0', token: process.env.BROK_TOKEN }),
);
socket.on('data', (data: Buffer) => {
    // One read may hold part of a frame, or several.
    buffered = Buffer.concat([buffered, data]);
    while (buffered.length >= 4 && buffered.length >= 4 + buffered.readUInt32BE(0)) {
        const end = 4 + buffered.readUInt32BE(0);
        handle(JSON.parse(buffered.subarray(4, end).toString()));
        buffered = buffered.subarray(end);
    }
});
socket.on('error', (error: Error) => {
    log(`bus connection failed: ${error.message}`);
    process.exit(1);
});
