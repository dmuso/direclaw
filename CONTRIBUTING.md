# Contributing

## Development Environment

Use `nix-shell` before any build, lint, or test command.

```bash
nix-shell
rustc --version
cargo --version
```

## Workflow

1. Read relevant spec sections in `docs/build/spec/`.
2. Make the smallest coherent change.
3. Add or update automated tests.
4. Run full quality gates:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
```

## Pull Requests

- Keep PRs focused and reviewable.
- Reference relevant task/spec documents.
- Include behavior notes and test evidence.
- Do not merge if CI is failing.

## Code and Safety Requirements

- Route channel-originated execution through orchestrator path.
- Preserve queue lifecycle semantics (`incoming -> processing -> outgoing`).
- Enforce workspace isolation and shared-workspace allowlists.
- Avoid placeholder or misleading operational behavior.

## Documentation Requirements

Any user-visible command, config, or runtime behavior change must update:

- `README.md`
- `docs/user-guide/*` when operator-facing
- `docs/build/spec/*` when normative behavior changes
