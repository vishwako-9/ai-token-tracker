# tokentracker-cli

Local-first AI coding token usage and cost tracker.

## Prerequisites

- [Rust](https://www.rust-lang.org/) and [Cargo](https://cargo.rust-lang.org/)
- Minimum version: `edition = "2021"` (per `Cargo.toml`)
- No external database needed — everything uses a bundled SQLite library

## Build & Install

```bash
cargo build --release
```

This produces a binary at `target/release/tokentracker-cli`.

Or install it to your PATH:

```bash
cargo install --path .
```

The release binary is standalone; `cargo install` puts `tokentracker-cli` on your shell PATH for easy invocation as `tokentracker-cli` (or just `tokentracker` depending on how the binary was named).

## Command Reference

| Command | Purpose | Flags |
|---------|---------|-------|
| `sync` | Sync usage data from local collectors | `--provider <P>`, `-p` |
| `summary` | Show usage summary | `--days <N>`, `-P` (provider filter), `--model <S>` |
| `daily` | Show daily usage breakdown | `--days <N> (default: 90)`, `-P` (provider), `--model <S>`, `--since <S>`, `--until <S>`, `--json`, `--all` |
| `weekly` | Show weekly usage breakdown (ISO weeks) | `--weeks <N> (default: 12)`, `-P` (provider), `--model <S>`, `--since <S>`, `--until <S>`, `--json`, `--all` |
| `monthly` | Show monthly usage breakdown | `--months <N> (default: 6)`, `-P` (provider), `--model <S>`, `--since <S>`, `--until <S>`, `--json`, `--all` |
| `detail` | Show recent records | `--model <S>`, `-P` (provider), `--since <S>`, `--until <S>`, `-P` (limit default: 50) |
| `models` | List known models and their pricing | `--provider <P>` |
| `status` | Show collector detection status | (no flags) |
| `config` | Manage configuration | `--set <KEY=VALUE>`, `--list` |
| `update-pricing` | Update model pricing from LiteLLM | (no flags) |
| `export` | Export usage data | `--format <S> (default: csv)`, `-o <S>`, `--days <N> (default: 30)` |
| `serve` | Serve the local web dashboard | `--port <N> (default: 7680)` |
| `tui` | **Launch the interactive terminal UI** | `--days <N> (default: 30)` |
| `reprice` | Recompute cost for all records from current pricing | (no flags) |
| `antigravity` | Count Antigravity requests per model/day | `--days <N> (default: 30)`, `--json` |
| `completions` | Generate shell tab-completion scripts | `--shell <SHELL> (bash\|elvish\|fish\|powershell\|zsh)` |

## TUI (Interactive Terminal UI)

Run `tokentracker tui` to enter the interactive terminal interface.

- **7 views**: Press number keys **1-7** to switch views (Summary, Daily, Weekly, Monthly, Recent, Models, Status)
- **Command palette**: Press `/` to open the command palette, type a command name for fuzzy-matching suggestions, press **Enter** to autocomplete and stay in the palette, press **Enter** again to submit the bare command
- **Mouse support**: Click/scroll with the mouse is supported
- **/theme command**: Type `/theme` bare to list available themes (`default`, `nord`, `dracula`), or `/theme <name>` to switch themes immediately
- **Key bindings**: `[Tab] next`, `[1-7] jump`, `[↑/↓] scroll`, `[PgUp/PgDn] page`, `[/] commands`, `[q] quit`, `[r] refresh`, `[mouse] click/scroll`

## Local-First

All data is stored in a local SQLite database — **no data leaves the machine**. The optional `update-pricing` command fetches current model pricing from LiteLLM for cost estimation, but all usage records remain on your machine.

## License

MIT