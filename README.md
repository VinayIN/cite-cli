# cite-cli

CLI tool for scaffolding, validating, building, and deploying news content to Supabase.

## Installation

### Quick install

(MacOS/Linux only)
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/VinayIN/cite-cli/releases/download/v0.1.0-alpha/cite-cli-installer.sh | sh
```
(Windows only)
```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/VinayIN/cite-cli/releases/download/v0.1.0-alpha/cite-cli-installer.ps1 | iex"
```

### From source

```bash
git clone https://github.com/VinayIN/cite-cli.git
cd cite-cli
cargo build --release
./target/release/cite-cli --help
```

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
