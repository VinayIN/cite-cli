# cite-cli

CLI tool for scaffolding, validating, building, and deploying podcast content to Supabase.

## Installation

### Quick install

(MacOS/Linux only)
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/VinayIN/cite-cli/releases/download/v0.1.0-alpha.1/cite-cli-installer.sh | sh
```
(Windows only)
```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/VinayIN/cite-cli/releases/download/v0.1.0-alpha.1/cite-cli-installer.ps1 | iex"
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
cite-cli init my-project
# edit metadata.yml and add content files
cite-cli validate --path my-project
cite-cli build --path my-project
cite-cli status --path my-project
cite-cli deploy --path my-project
```

## Tests

```bash
cargo test
```

## Commands

| Command | Description |
|---|---|
| `init <name>` | Scaffold a new project |
| `validate` | Check structure, metadata, file existence |
| `lint` | Word counts and content quality checks |
| `build` | Incremental build -> `build/content.json` |
| `deploy` | Deploy to Supabase (full JSON to storage + table subset) |
| `status` | Project health overview |
| `doctor` | Diagnose config and structure issues |
| `clean` | Remove build artifacts and cache |
| `rollback <id>` | Undo a deployment by ID |

All commands accept `--path <dir>` to target a specific directory.
Without `--path`, projects are auto-discovered in the current directory and subdirectories.

## Project Structure

```
my-project/
├── cite.toml           # Project manifest
├── metadata.yml        # Podcast content metadata
├── content/            # Markdown & BibTeX content files
│   ├── article1.md     
│   └── article1.bib
├── assets/
│   ├── audio/          # Podcast audio files
│   └── image/          # Thumbnails and cover art
└── build/              # Auto-Generated build output (gitignored)
```
