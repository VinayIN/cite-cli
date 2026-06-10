# cite-cli

CLI tool for scaffolding, validating, building, and deploying news content to Supabase.

## Installation

### Quick install

```bash
curl -fsSL https://raw.githubusercontent.com/VinayIN/cite-cli/main/install.sh | sh
```

> **Note:** To install to `/usr/local/bin` (instead of `~/.local/bin`), run as root

The script will:
1. Detect your OS and architecture
2. Download a pre-built binary from GitHub Releases (if available), or build from source via Cargo
3. Install to `~/.local/bin` (or `/usr/local/bin` if run as root)
4. Add the install directory to your `PATH` in your shell config (`.bashrc`/`.zshrc`)

> **Note:** Building from source requires [Rust](https://rustup.rs). If no pre-built binary exists for your platform and Rust is not installed, the script will prompt you to install it.

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
