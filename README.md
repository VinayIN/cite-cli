# cite-cli

CLI tool for scaffolding, validating, building, and deploying news content to Supabase.

## Usage

```bash
cargo run init my-project
cargo run validate --path my-project
cargo run build --path my-project
cargo run status --path my-project
```

## Tests

```bash
cargo test
```
## Build a release binary:

```bash
cargo build --release
./target/release/cite-cli --help
```
and then can use this:
```
./target/release/cite-cli init my-project
./target/release/cite-cli validate
```

## Commands

| Command | Description |
|---|---|
| `init <name>` | Scaffold a new project |
| `validate` | Check structure, metadata, cross-refs, file existence |
| `lint` | Naming conventions, audio metadata, word counts |
| `build` | Incremental build → `build/content.json` |
| `deploy` | Upsert to Supabase staging with rollback support |
| `status` | Project health overview |
| `doctor` | Diagnose config and structure issues |
| `clean` | Remove build artifacts and cache |

All commands accept `--path <dir>` to target a specific directory.
