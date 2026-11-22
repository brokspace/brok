#!/usr/bin/env python3
"""Generate Homebrew formulae from published Brokspace GitHub release assets."""

from __future__ import annotations

import argparse
import re
import urllib.request
from pathlib import Path

PLATFORMS = (
    ("macos-aarch64", "on_macos do\n    on_arm do"),
    ("macos-x86_64", "on_macos do\n    on_intel do"),
    ("linux-aarch64", "on_linux do\n    on_arm do"),
    ("linux-x86_64", "on_linux do\n    on_intel do"),
)


def checksum(repo: str, tag: str, asset: str) -> str:
    url = f"https://github.com/{repo}/releases/download/{tag}/{asset}.sha256"
    request = urllib.request.Request(url, headers={"User-Agent": "brokspace-homebrew-updater"})
    with urllib.request.urlopen(request, timeout=30) as response:
        value = response.read().decode("ascii").strip().split()[0].lower()
    if not re.fullmatch(r"[0-9a-f]{64}", value):
        raise ValueError(f"Invalid SHA-256 sidecar at {url}")
    return value


def formula(class_name: str, formula_name: str, binary: str, description: str,
            repo: str, tag: str, version: str) -> str:
    blocks = []
    for platform, condition in PLATFORMS:
        asset = f"{binary}-{platform}"
        sha = checksum(repo, tag, asset)
        outer, inner = condition.split("\n    ")
        blocks.append(
            f"  {outer}\n    {inner}\n"
            f"      url \"https://github.com/{repo}/releases/download/{tag}/{asset}\", using: :nounzip\n"
            f"      sha256 \"{sha}\"\n"
            f"    end\n  end"
        )
    platform_blocks = "\n\n".join(blocks)
    return f'''class {class_name} < Formula
  desc "{description}"
  homepage "https://brokspace.org"
  version "{version}"
  license any_of: ["MIT", "Apache-2.0"]

{platform_blocks}

  def install
    candidates = Dir.glob("{binary}-*").select {{ |path| File.file?(path) }}
    odie "Expected one downloaded {binary} executable, found #{{candidates.length}}" unless candidates.length == 1
    bin.install candidates.first => "{binary}"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/{binary} --version")
  end
end
'''


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tap", required=True, type=Path)
    parser.add_argument("--brok-tag", required=True)
    parser.add_argument("--agent-tag", required=True)
    args = parser.parse_args()

    for tag in (args.brok_tag, args.agent_tag):
        if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", tag):
            raise SystemExit(f"Invalid release tag: {tag}")

    (args.tap / "Formula").mkdir(parents=True, exist_ok=True)
    args.tap.joinpath("Formula/brok.rb").write_text(
        formula("Brok", "brok", "brok", "Local-first developer workspace search and agents",
                "brokspace/brok", args.brok_tag, args.brok_tag[1:]), encoding="utf-8")
    (args.tap / "Formula/brok-agent.rb").write_text(
        formula("BrokAgent", "brok-agent", "brok-agent", "Default Brokspace agent and shard runtime",
                "brokspace/brok-agent", args.agent_tag, args.agent_tag[1:]), encoding="utf-8")
    print(f"Updated formulae in {args.tap / 'Formula'}")


if __name__ == "__main__":
    main()
