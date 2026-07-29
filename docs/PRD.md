## 1. Product Overview

**cite-cli** is a production-grade CLI tool for creating, validating, building, and deploying podcast content to a Supabase backend consumed by the aouxAI application.

The CLI manages the complete content lifecycle:

* project initialization
* content validation
* deterministic compilation
* media processing
* remote deployment to Supabase
* rollback
* local analytics
* interactive terminal workflows

The system is designed around a simple project structure, reproducible builds, safe deployments, and offline-first local state. All deployments are remote to Supabase; local state maintains deployment history for rollback and analytics.

---

# 2. Objectives

* Provide consistent project scaffolding.
* Separate content, metadata, and deployment concerns.
* Ensure content and media quality through automated validation.
* Produce deterministic and incremental builds.
* Safely deploy content with verification and rollback.
* Maintain local analytics, history, and compiler cache.
* Support interactive TUI workflows and CI automation.
* Treat podcast text, audio, and media assets as first-class content.

---

# 3. Core Concepts

## Project

A project is a directory containing:

* `cite.toml`
* `metadata.yml`
* content files
* media assets

---

## Artist

An existing Supabase user identified by UUID.

Configured in:

```toml
[project]
artist_id = "..."
```

The CLI user is the content creator.

---

## Podcast

A podcast item consists of:

* Markdown content
* Optional audio
* Optional thumbnail
* Optional source URL
* Optional BibTeX citation

Each podcast receives a persistent UUID during compilation.

---

## Compiler Protocol

A versioned compiler transforms project files into a deployable bundle:

```
project files → build/content.json
```

The compiler guarantees:

* deterministic output
* incremental processing
* persistent identity
* asset metadata extraction

---

## Deployment

A deployment:

* uploads the complete bundle to Supabase
* synchronizes database records
* creates searchable/queryable data
* records deployment history

Every deployment receives a unique deployment ID.

---

# 4. Metadata Model (`metadata.yml`)

```yaml
podcasts:
  - title: "My Podcast"
    file: content/my-article.md
    source_url: "https://example.com"
    category: "artificial intelligence"
    audio: assets/audio/episode.mp3
    thumbnail: assets/image/thumb.jpg
    citation: content/my-article.bib
```

Rules:

* `artist_id` remains in `cite.toml`
* podcasts are defined as an array
* markdown remains separate from metadata
* no slugs
* no audio tiers
* no relationship metadata
* relationships are resolved during deployment
* audio, thumbnail, source_url, and citation are optional fields

---

# 5. CLI Commands

| Command                    | Description                                     |
| -------------------------- | ----------------------------------------------- |
| `init <name>`              | Create project structure                        |
| `doctor`                   | Validate project, metadata, content, and assets |
| `lint`                     | Analyze content and media quality               |
| `build`                    | Compile project into `build/content.json`       |
| `deploy`                   | Upload bundle and synchronize backend           |
| `status`                   | Show project health and analytics               |
| `clean`                    | Remove build artifacts and cache                |
| `rollback <deployment-id>` | Remove deployment data from Supabase            |
| `login`                    | Authenticate and link account                   |
| `upgrade`                  | Update CLI                                      |
| `uninstall`                | Remove CLI                                      |

### Interactive Terminal UI

Running `cite-cli` with no command-line arguments (or with `--tui` flag) enters interactive ratatui-based terminal interface.

Global options:

```
--path <path>               Path to project (default: current directory)
--config <path>             Path to credentials file (default: ~/.cite/credentials.toml)
--json                      Machine-readable JSON output
--quiet                     Suppress output
--verbose                   Detailed output
--dry-run                   Preview changes without executing
```

Commands provide stable exit codes for automation.

---

# 6. Build Pipeline

```
Project
   |
Load Context
   |
Resolve Paths
   |
Validate Metadata
   |
Validate Assets
   |
Extract Media Metadata
   |
Parse Markdown
   |
Parse BibTeX
   |
Resolve URLs
   |
Assign Persistent UUIDs
   |
Generate content.json
   |
Update Cache
```

Compiler responsibilities:

* generate deterministic output
* maintain persistent UUID mapping
* embed markdown content
* parse citations
* extract audio metadata (duration, format, codec, bitrate)
* extract image metadata (format, dimensions, file size)
* calculate asset hashes
* perform incremental builds
* invalidate cache on compiler version change

---

## Media Metadata

### Audio Assets

Extracted during build:

* duration (in seconds)
* format (MP3, WAV, FLAC, etc.)
* codec
* bitrate (kbps)
* sample rate (Hz)
* channels
* file size (bytes)
* checksum (SHA-256)

Validation requirements:

* duration > 0
* format is supported (MP3, WAV, FLAC, OGG, M4A)
* file is readable

### Image Assets

Extracted during build:

* format (JPEG, PNG, WebP, etc.)
* dimensions (width × height in pixels)
* file size (bytes)
* checksum (SHA-256)

Validation requirements:

* format is supported (JPEG, PNG, WebP, GIF)
* file is readable
* dimensions >= 100×100 pixels (minimum)
* file size <= 5 MB

### Binary File Handling

Binary files are not stored in local analytics; only metadata and checksums are tracked.

---

# 7. Deployment Pipeline

```
content.json
      |
Verify Supabase connectivity
      |
Upload bundle
      |
Verify upload
      |
Process podcasts
      |
For each podcast:
  - Verify artist
  - Resolve category
  - Resolve URL/domain
  - Create news record
  - Create artist relationship
  - Create metrics
  - If audio present: upload audio
  - If thumbnail present: upload thumbnail
  - Create timeline records
      |
Record deployment history
      |
Verify deployment complete
```

Deployment guarantees:

* deployment ID tagging
* dry-run support
* transactional writes where supported
* retry handling for failed uploads
* verification after completion
* rollback support by deployment ID

Deployment error handling:

* If deployment fails partway through, record partial state in local history
* `--dry-run` previews all actions without uploading
* Failed uploads retry up to 3 times before aborting

---

# 8. Project Structure

```
my-project/
├── cite.toml
├── metadata.yml
├── .gitignore
├── content/
├── assets/
│   ├── audio/
│   └── image/
└── build/
```

This structure is fixed. The `.cite/` directory (created automatically) contains:

```
.cite/
├── analytics.db          # Local SQLite database
├── credentials.toml      # (Optional: project-level credentials)
└── cache/                # Compiler cache
```

---

# 9. Local Analytics Database

cite-cli maintains a local SQLite database at `.cite/analytics.db` for offline analytics, caching, and history. The database is created during `init` and updated by `build` and `deploy` commands. It is portable and human-inspectable.

Database schema is initialized on first run and migrated automatically on CLI upgrades.

---

## Stored Data

### Projects

* project metadata
* local path
* artist ID
* language
* synchronization state with Supabase

---

### Podcasts

* metadata
* compiled content
* word count
* reading time
* audio metadata (if present)
* thumbnail metadata (if present)
* timestamps
* deployment status

---

### Assets

Tracks:

* asset type (audio, image)
* file path
* checksum (SHA-256)
* size (bytes)
* metadata (duration, dimensions, codec, etc.)
* usage (which podcast uses this asset)

---

### Timeline Entries

* BibTeX-derived references
* linked podcast
* timestamps

---

### Build History

* compiler version
* build duration (seconds)
* build statistics (podcast count, total word count, etc.)
* incremental build flag
* timestamp

---

### Deployment History

* deployment ID
* counts (podcasts deployed, assets uploaded)
* status (success, partial, failed)
* timestamps (start, end)
* dry-run flag
* error messages (if failed)

---

### Compiler Cache

* file hashes (SHA-256)
* UUID mappings (file → persistent podcast UUID)
* compiler version (for cache invalidation)

---

# 10. Analytics Capabilities

Provides:

* project statistics (podcast count, total word count)
* podcast statistics (per-podcast metadata)
* word counts (total and per-podcast)
* reading time estimates (total and per-podcast)
* total audio duration
* average episode duration
* media storage usage (total audio size, total image size)
* asset statistics (count, types, sizes)
* citation statistics (count, references)
* build history (last build date, build count, average duration)
* deployment history (last deployment, deployment count, status summary)
* offline analytics (all analytics available without network)

---

# 11. Interactive Terminal UI

Running `cite-cli` with no arguments (or `--tui` flag) opens a ratatui-based interface.

Modes:

* Main View
* Analytics View
* Explorer View
* History View

---

## Main View

Four panels:

1. Projects
2. Commands
3. Details
4. Logs

Controls:

| Key        | Action         |
| ---------- | -------------- |
| Tab        | Next panel     |
| Shift+Tab  | Previous panel |
| Arrow keys | Navigate       |
| Enter      | Execute        |
| r          | Refresh        |
| PgUp/PgDn  | Scroll         |
| Esc        | Exit           |

---

## Analytics View (`s`)

Two-panel layout:

1. Projects
2. Analytics

In the Analytics pane, it displays:

* podcast count
* word count
* reading time
* total audio duration
* average episode duration
* timeline entries
* builds (count, last build date)
* deployments (count, last deployment date)
* assets (count, storage usage)

Navigation:

* ↑/↓/←/→ Navigate
* Tab switch panels
* r refresh
* Esc return

---

## Explorer View (`e`)

Two-panel layout:

1. Projects
2. Contents

The Content pane displays:

* overview (project name, artist, podcast count)
* podcasts (list with titles and metadata)
* timelines (timeline entries)
* builds (build history summary)
* deployments (deployment history summary)

Navigation:

* ↑/↓/←/→ Navigate
* Tab switch panels
* r refresh
* Esc return

---

## History View (`h`)

Three-panel layout:

* Project (list)
* Build history
* Deployment history

Navigation:

* ↑/↓ Navigate between projects
* Tab switch panels
* r refresh
* Esc return

---

## Global Navigation

| Key | View      |
| --- | --------- |
| m   | Main      |
| s   | Analytics |
| e   | Explorer  |
| h   | History   |
| q   | Quit      |

Long-running tasks (build, deploy) execute asynchronously with live progress updates in the Logs panel.

---

# 12. Validation

## Doctor

Validates project integrity and reports errors (must fix before deploy). Runs on `doctor` and before `build`/`deploy`.

### Project

* structure (cite.toml, metadata.yml, content/, assets/ exist)
* configuration (valid cite.toml syntax)
* required files present

### Metadata

* required fields (title, file for each podcast)
* duplicate podcast titles
* invalid values (empty strings, invalid UUIDs)
* unknown keys (warn on unexpected fields)
* file references exist

### Markdown

* empty content (file exists and is non-empty)
* syntax errors (invalid YAML frontmatter if present)
* broken internal references

### Audio (if specified)

* supported formats (MP3, WAV, FLAC, OGG, M4A)
* file readability (file exists and is readable)
* duration > 0 seconds
* file size <= 500 MB

### Images (if specified)

* supported formats (JPEG, PNG, WebP, GIF)
* file readability (file exists and is readable)
* dimensions >= 100×100 pixels
* file size <= 5 MB

### BibTeX (if specified)

* parser errors (valid BibTeX syntax)
* duplicate keys
* invalid fields

### URLs (if specified)

* basic validity (well-formed URLs)
* no duplicates within project

Output levels:

* **Error:** Blocks build/deploy (must fix)
* **Warning:** Does not block but should be reviewed
* **Information:** Informational only

---

# 13. Linting

Analyzes content and media quality. Lint warnings do not block builds but should be reviewed.

## Content Quality

* word count (warn if < 100 or > 50,000 words)
* reading time (estimated from word count)
* heading consistency (document should have H1/H2 structure)
* paragraph quality (warn on very short paragraphs < 20 words)
* citation density (warn if no citations in long content)
* duplicate sections (detect repeated paragraphs)

## Media Quality

* missing audio (info if none specified, since audio is optional)
* unusual duration (warn if < 1 min or > 4 hours)
* inconsistent encoding (warn if audio format differs from majority)
* oversized files (warn if audio > 200 MB, image > 3 MB)
* image quality (warn if dimensions < 200×200 or > 8000×8000)
* audio bitrate (warn if < 128 kbps or > 320 kbps)
* sample rate inconsistency (warn if project has mixed sample rates)

---

# 14. Authentication & Credentials

### Login Flow

`cite-cli login` prompts for:
* Supabase project URL
* Supabase API key (public/anon key)

Credentials are stored at `~/.cite/credentials.toml`:

```toml
[supabase]
url = "https://your-project.supabase.co"
api_key = "eyJhbG..."
```

Alternatively, credentials can be provided via environment variables:

```bash
CITE_SUPABASE_URL="https://your-project.supabase.co"
CITE_SUPABASE_API_KEY="eyJhbG..."
cite-cli deploy
```

Credentials are never stored in `cite.toml` or project directories.

---

# 15. Design Principles

* deterministic builds (same input → same output)
* incremental compilation (unchanged files skip reprocessing)
* immutable deployment bundles (deployments cannot be modified, only rolled back)
* metadata/content separation (no frontmatter in markdown)
* UUID-based identity (persistent podcast IDs across rebuilds)
* automatic relationship resolution (at deployment time, not build time)
* local-first analytics (all analytics work offline)
* media-aware processing (audio/image metadata extracted automatically)
* reproducible compiler output (same files + same compiler version = same JSON)
* remote-only deployment (all uploads go to Supabase, no local staging)
* database-independent storage (SQLite is abstracted via standard SQL)

---

# 16. Reliability Requirements

cite-cli provides:

* deterministic compiler output (byte-for-byte identical builds)
* persistent UUID assignment (podcast UUIDs survive rebuild/redeploy)
* incremental builds (only reprocess changed files)
* deployment verification (verify all assets uploaded before marking success)
* rollback by deployment ID (remove specific deployment from Supabase)
* compiler protocol versioning (incompatible versions invalidate cache)
* structured machine-readable output (--json flag on all commands)
* safe failure handling (partial deployments logged; no silent failures)
* media integrity verification (checksums verify audio/image integrity)
* dry-run support (--dry-run previews all actions without uploading)
* retry logic (automatic retry on transient failures)
* offline capability (build/lint/doctor work without network; only deploy/login require network)
