"""Recall and size of smaller vector formats, measured on a real brok index.

    pip install numpy pyarrow onnxruntime tokenizers
    brok config set index.store arrow && brok index --rebuild
    python3 scripts/vector_study.py <workspace> <model folder>

The model folder is the one `brok model status` shows. Read only. See
docs/adr/0017-compact-vector-storage.md.
"""
import glob, os, re, sys
import numpy as np, pyarrow as pa, onnxruntime as ort
from tokenizers import Tokenizer
workspace, M = sys.argv[1], os.path.join(sys.argv[2], '')
t = pa.ipc.open_file(sorted(glob.glob(workspace + '/.brokspace/databases/cold/*.arrow'))[-1]).read_all().combine_chunks()
D = np.asarray(t.column('vector').chunk(0).flatten(), dtype=np.float32).reshape(t.num_rows, 768)
paths = t.column('path').to_pylist()
rows = {c: t.column(c).to_pylist() for c in ('parent', 'symbol', 'content')}
tok = Tokenizer.from_file(M + 'tokenizer.json'); tok.enable_truncation(512)
sess = ort.InferenceSession(M + 'model_quantized.onnx', providers=['CPUExecutionProvider'])
names = [i.name for i in sess.get_inputs()]
def embed(text):
    e = tok.encode(text)
    ids = np.array([e.ids], np.int64); mask = np.array([e.attention_mask], np.int64)
    feed = {'input_ids': ids, 'attention_mask': mask, 'token_type_ids': np.zeros_like(ids)}
    h = sess.run(None, {n: feed[n] for n in names})[0][0]
    p = h[mask[0] == 1].mean(0); p = (p - p.mean()) / np.sqrt(p.var() + 1e-5)
    return (p / np.linalg.norm(p)).astype(np.float32)
# Check: this reproduces what brok stored.
sims = []
for i in range(0, len(paths), 211):
    text = paths[i] + ''.join(' > ' + rows[k][i] for k in ('parent', 'symbol') if rows[k][i] is not None) + '\n' + rows['content'][i]
    sims.append(float(embed('search_document: ' + text) @ D[i]))
print(f'harness check, cosine with stored vectors: min {min(sims):.5f} over {len(sims)} chunks')

queries = [
 ('where is the event bus listening', r'brok-bus/src/(lib|session)\.rs'), ('how are secrets masked before leaving the machine', r'redact|guardrails|secret'),
 ('incremental indexing of changed files', r'brok-index/src/(index|service)\.rs'), ('install a shard from a git url', r'brok-shard/src/install'),
 ('search the whole obsidian vault', r'vault\.rs|obsidian'), ('BM25 scoring formula', r'bm25\.rs'),
 ('wait until the daemon is ready', r'daemon\.rs'), ('download the embedding model', r'embed/mod\.rs|cmd/model\.rs'),
 ('which topics may a shard publish', r'auth\.rs|manifest\.rs|0003'), ('save a memory shared with the team', r'memory'),
 ('search linked workspaces', r'links?\.rs'), ('split rust code into chunks with tree sitter', r'chunk/'),
 ('upgrade to the latest release', r'upgrade\.rs'), ('environment variables a shard receives', r'sandbox\.rs|env\.rs'),
 ('track git changes in the workspace', r'tracker\.rs'), ('stream the answer from the language model', r'llm\.rs|ask\.rs'),
 ('watch the file system for edits', r'watch\.rs'), ('limit how many questions run at once', r'queue\.rs'),
 ('framing of messages on the socket', r'codec\.rs'), ('attach open editor tabs to the chat', r'context\.ts|composer'),
 ('first run onboarding checklist', r'onboarding'), ('cancel a running search in the editor', r'search\.ts|latest\.ts'),
 ('download and verify the brok binary from the extension', r'installer'), ('remember chat sessions and resume them', r'sessions?\.ts|history'),
 ('sync documentation into the website', r'sync-docs'), ('mirror a remote vault over https', r'brok-obsidian/src'),
 ('validate shard.json fields', r'manifest\.rs|shard\.schema'), ('merge results from several partitions', r'search\.rs'),
 ('record local metrics about searches', r'metrics\.rs'), ('connect coding agents through mcp', r'mcp\.rs|integrations'),
]
Q = np.stack([embed('search_query: ' + q) for q, _ in queries])

def norm(v): return v / np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-12)
def f32(d): return lambda v: norm(v[:, :d])
rng = np.random.default_rng(7); sample = rng.choice(len(D), 500, replace=False)
def scores(kind, d, q):
    docs = norm(D[:, :d]); q = norm(q[:, :d])
    if kind == 'float32': return q @ docs.T
    if kind == 'float16': return q @ docs.astype(np.float16).astype(np.float32).T
    if kind == 'int8':      # one scale per vector, stored as 4 extra bytes
        scale = np.abs(docs).max(1, keepdims=True) / 127
        return q @ (np.round(docs / scale).astype(np.int8).astype(np.float32) * scale).T
    if kind == 'binary': return np.sign(q) @ np.sign(docs).T / d
def top(s, k, own=None):
    s = s.copy()
    if own is not None: s[np.arange(len(own)), own] = -np.inf
    return np.argsort(-s, axis=1, kind='stable')[:, :k]
base_q = top(scores('float32', 768, Q), 32); base_d = top(scores('float32', 768, D[sample]), 10, sample)
size = {'float32': lambda d: 4 * d, 'float16': lambda d: 2 * d, 'int8': lambda d: d + 4, 'binary': lambda d: d // 8}
print(f'{len(D)} chunks, {len(queries)} questions, 500 chunk-as-query samples')
print('| storage | dims | bytes/vector | vs float32 768 | recall@8 | recall@32 | chunk recall@10 | expected file in top 8 | MRR |')
print('|---|---:|---:|---:|---:|---:|---:|---:|---:|')
for d in (768, 512, 256, 128):
    for kind in ('float32', 'float16', 'int8', 'binary'):
        a = top(scores(kind, d, Q), 32)
        r32 = np.mean([len(set(x) & set(y)) / 32 for x, y in zip(a, base_q)])
        r8 = np.mean([len(set(x[:8]) & set(y[:8])) / 8 for x, y in zip(a, base_q)])
        b = top(scores(kind, d, D[sample]), 10, sample)
        r10 = np.mean([len(set(x) & set(y)) / 10 for x, y in zip(b, base_d)])
        ranks = [next((n + 1 for n, i in enumerate(x) if re.search(p, paths[i])), None) for x, (_, p) in zip(a, queries)]
        hit = sum(1 for r in ranks if r and r <= 8); mrr = np.mean([1 / r if r else 0 for r in ranks])
        b_ = size[kind](d)
        print(f'| {kind} | {d} | {b_} | {b_ / 3072:.0%} | {r8:.1%} | {r32:.1%} | {r10:.1%} | {hit}/{len(queries)} | {mrr:.3f} |')
# Binary first pass, rescored with int8 (both stored).
b_scores = scores('binary', 768, Q); i_scores = scores('int8', 768, Q)
cand = top(b_scores, 128)
rescored = np.stack([c[np.argsort(-i_scores[n, c], kind='stable')][:32] for n, c in enumerate(cand)])
print('binary 768 (top 128) rescored by int8 768: recall@32 %.1f%%' % (100 * np.mean([len(set(x) & set(y)) / 32 for x, y in zip(rescored, base_q)])))
