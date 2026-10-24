# Test examples

## Behavior through the public interface

Rust, good: the test exercises `Index::search`, the seam users depend on.

```rust
#[tokio::test]
async fn finds_functions_by_concept() {
    let ws = workspace_with(&[("src/auth.rs", "pub fn validate_token(t: &str) -> bool { !t.is_empty() }")]);
    let mut index = Index::open(&ws, None, Access::ReadWrite).await.unwrap();
    index.index_all(false, &|_| {}).await.unwrap();

    let hits = index.search(&SearchQuery { text: "validate token".into(), limit: 1, focus: None }).await.unwrap();

    assert_eq!(hits.hits[0].symbol.as_deref(), Some("validate_token"));
}
```

Rust, bad: reaches into a private tokenizer and asserts its internals.

```rust
#[test]
fn tokenizer_internal_vec_has_capacity() {
    let t = Tokenizer::new();
    assert!(t.buffer.capacity() >= 64); // breaks on any refactor, proves nothing for users
}
```

TypeScript, good: test the module's exported function with real input.

```ts
test("formats durations under a minute in seconds", () => {
  expect(formatDuration(42_000)).toBe("42s");
});
```

TypeScript, bad: mocks a collaborator and asserts it was called.

```ts
test("formatDuration calls padStart", () => {
  const spy = vi.spyOn(String.prototype, "padStart");
  formatDuration(42_000);
  expect(spy).toHaveBeenCalled(); // implementation detail, not behavior
});
```

## Independent expected values

Bad, tautological (TypeScript):

```ts
expect(total(items)).toBe(items.reduce((sum, i) => sum + i.price * i.qty, 0));
```

Good, a worked example:

```ts
expect(total([{ price: 250, qty: 2 }, { price: 100, qty: 1 }])).toBe(600);
```

## Regression tests for bugs

Reproduce the reported input exactly, assert the correct outcome, and name the
test after the behavior, not the ticket number:

```rust
#[test]
fn markdown_headings_inside_code_fences_are_not_sections() {
    let md = "# Title\n```sh\n# not a heading\n```\n";
    let sections = chunk_markdown(md);
    assert_eq!(sections.len(), 1);
}
```

## Fakes at the seam

When a dependency is slow or external (an LLM, a network API), fake the whole
dependency at its interface, with the smallest behavior the test needs:

```rust
struct FixedEmbedder; // implements the Embedder trait, returns deterministic vectors
```

Do not fake parts of the code under test.
