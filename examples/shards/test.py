#!/usr/bin/env python3
"""Installs the example shards into a temporary workspace and checks what they
publish: `python3 examples/shards/test.py path/to/brok [python typescript java]`.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

EXAMPLES = Path(__file__).resolve().parent
TRICKY = 'src/a "quoted" é.rs'


def main():
    brok = str(Path(sys.argv[1]).resolve())
    languages = sys.argv[2:] or ["python", "typescript", "java"]
    with tempfile.TemporaryDirectory(prefix="brok-examples-") as temp:
        temp = Path(temp)
        workspace = temp / "workspace"
        env = dict(os.environ, BROK_CONFIG_DIR=str(temp / "config"), BROK_MODELS_DIR=str(temp / "models"))

        def run(*args):
            done = subprocess.run([brok, *args], cwd=workspace, env=env, capture_output=True, text=True, timeout=180)
            if done.returncode:
                sys.exit(f"brok {' '.join(args)} failed:\n{done.stdout}{done.stderr}")
            return done.stdout

        (workspace / "src").mkdir(parents=True)
        # A workspace may declare either kind of JavaScript; the shard must not care.
        (workspace / "package.json").write_text('{"type": "module"}\n')
        (workspace / TRICKY).write_text("fn compute_invoice_total() {}\n// TODO: round the total\n", encoding="utf-8")
        (temp / "outside.txt").write_text("TODO: secret\n")
        run("init", "--yes", "--no-agent", "--skip-model", "--no-llm", "--no-integrations")
        for language in languages:
            copy = temp / language
            shutil.copytree(EXAMPLES / language, copy)
            run("shard", "install", str(copy), "--yes")
        names = [f"todo-{language}" for language in languages]
        tail_file = temp / "tail.txt"
        run("start")
        tail = None
        try:
            logs = workspace / ".brokspace" / "logs"
            wait(lambda: all("ready" in read(logs / f"{n}.log") for n in names), "the shards to be ready", logs)
            with open(tail_file, "w") as out:
                tail = subprocess.Popen([brok, "bus", "tail", "*"], cwd=workspace, env=env, stdout=out)
            time.sleep(1)
            run("bus", "emit", "file:changed", json.dumps({"changed": [TRICKY, "../outside.txt", "gone.rs"], "deleted": []}))
            for name in names:
                run("bus", "emit", f"{name}:ask", json.dumps({"query": "invoice total"}))
            for name in names:
                wait(lambda: published(tail_file, name, "answer"), f"{name} to answer", logs)
                todo = published(tail_file, name, "todo")
                assert todo == [{"path": TRICKY, "line": 2, "text": "// TODO: round the total"}], (name, todo)
                answer = published(tail_file, name, "answer")
                # A file may match with more than one chunk.
                assert len(answer) == 1 and set(answer[0]["paths"]) == {TRICKY}, (name, answer)
                print(f"ok {name}")
        finally:
            if tail:
                tail.kill()
            run("stop")
        for name in names:
            log = read(logs / f"{name}.log")
            assert "bus error" not in log, log
    print("all examples work")


def read(path):
    return path.read_text(encoding="utf-8", errors="replace") if path.exists() else ""


def published(tail_file, name, kind):
    """Payloads the shard published on `<name>:<kind>`, from `brok bus tail`."""
    marker = f" {name}:{kind} from {name} "
    return [json.loads(line.split(marker, 1)[1]) for line in read(tail_file).splitlines() if marker in line]


def wait(condition, what, logs):
    deadline = time.time() + 60
    while not condition():
        if time.time() > deadline:
            shown = "\n".join(f"--- {p.name}\n{read(p)[-2000:]}" for p in logs.glob("*.log"))
            sys.exit(f"timed out waiting for {what}\n{shown}")
        time.sleep(0.2)


if __name__ == "__main__":
    main()
