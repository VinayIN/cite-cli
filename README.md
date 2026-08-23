## Installation

### Quick install

(MacOS/Linux only)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/VinayIN/cite-cli/releases/download/v0.1.0-alpha.3/cite-cli-installer.sh | sh
```

(Windows only)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/VinayIN/cite-cli/releases/download/v0.1.0-alpha.3/cite-cli-installer.ps1 | iex"
```

### From source

```bash
git clone https://github.com/VinayIN/cite-cli.git
cd cite-cli
cargo build --release
./target/release/cite-cli --help
```

## Quick Start

```bash
cite-cli init my-project
# edit metadata.yml and add content files
cite-cli doctor --path my-project
cite-cli build --path my-project
cite-cli login
cite-cli deploy --path my-project
```

## Interactive Terminal UI

Run `cite-cli` with no arguments to enter the TUI:

| Key                 | Action                        |
| ------------------- | ----------------------------- |
| `Ctrl+P`            | Toggle command palette        |
| `Tab` / `Shift+Tab` | Cycle focus between panels    |
| `↑` / `↓`           | Navigate lists                |
| `Enter`             | Execute command / select file |
| `Ctrl+r`            | Refresh project list          |
| `←` / `→`           | Navigate commands             |
| `Ctrl+q`            | Quit                          |

> When a command with arguments is selected, type args in the Details panel then press `Enter` to execute.

## Commands

| Command                    | Description                                                                                 |
| -------------------------- | ------------------------------------------------------------------------------------------- |
| `init <name>`              | Create project structure                                                                    |
| `doctor`                   | Validate project, metadata, files, assets, config, content quality, and media; show project health and local analytics |
| `build`                    | Compile project → `build/content.json` (incremental)                                        |
| `deploy`                   | Upload bundle to Supabase with verification                                                 |
| `deploy --staging`         | Deploy to local cite.db instead of Supabase                                                 |
| `clean`                    | Remove build artifacts and cache                                                            |
| `rollback <deployment-id>` | Roll back to a previous deployment                                                          |
| `login`                    | Authenticate with Supabase credentials                                                      |
| `upgrade`                  | Self-update CLI                                                                             |
| `uninstall`                | Remove CLI                                                                                  |

> Global options (all commands): `--path <dir>`, `--json`, `--quiet`, `--verbose`, `--dry-run`.
> Command-specific: `doctor --json`, `build --force`, `deploy --staging`, `login --email --password`, `rollback <id>`, `uninstall --force`.

## Project Structure

```
my-project/
├── cite.toml           # Project manifest (artist_id UUID)
├── metadata.yml        # Podcast metadata
├── content/            # Markdown & BibTeX files
├── assets/
│   ├── audio/          # Podcast audio (optional)
│   └── image/          # Thumbnails (optional)
└── build/              # Generated output (gitignored)
```

## Metadata Model

```yaml
podcasts:
  - title: "My Podcast"
    file: content/my-article.md
    source_url: "https://example.com"
    category: "artificial intelligence"
    audio: assets/audio/episode.mp3 # optional
    thumbnail: assets/image/thumb.jpg # optional
    citation: content/my-article.bib # optional
```

## Local Analytics

cite-cli maintains a local database at `~/.cite/cite.db` for:

- Compiler cache (file hashes, UUID mappings)
- Build and deployment history
- Project and podcast statistics (word count, reading time, audio duration)
- Asset metadata and usage tracking
- Offline analytics — no network required

## Tests

```bash
cargo test
```
