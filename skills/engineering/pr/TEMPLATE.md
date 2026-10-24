# Pull request template

```markdown
## Summary

<The smallest visual that makes the change clear: two sentences, a
before/after table, or a short transcript.>

| Before | After |
|---|---|
| `brok search` loads the model on every call | `brok search` reuses the daemon's model |

Closes #<issue>

## Evidence

- `cargo test -p brok-index` passes, including the new
  `searches_reuse_the_daemon_model` test.
- Manual check:
  ```console
  $ brok search "token validation"
  0.97 src/auth.rs:42-77 function validate_token
  ```

## Risk

- Door: two-way (no format or protocol change; revert is safe).
- Blast radius: `brok search` only; on failure it falls back to opening the
  index directly.

## Review guide

Start with `crates/brok-cli/src/cmd/search.rs`; `daemon.rs` is plumbing.
```

Drop the review guide for small changes. Never leave a section with only a
placeholder: delete it or fill it.
