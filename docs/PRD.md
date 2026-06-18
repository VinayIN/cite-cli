# cite-cli — Product Requirements Document

## 1. Product Overview
**cite-cli** is a command-line tool that enables users to create, structure, validate, build, and deploy news content into a **Supabase staging backend** consumable by the **aouxAI** application. This tool is designed to be used inside IDEs and editors to manage content projects. It provides a complete lifecycle for a content project from scaffolding through deployment to the staging environment.

---

## 2. Objectives
- Accelerate project initialization with consistent, high-quality scaffolding.
- Ensure content quality, consistency, and structural integrity through automated linting and validation.
- Provide a reliable build/compile process with an explicit, versioned protocol.
- Enable safe, isolated deployment to a Supabase staging backend.
- Make complex content projects (mixed text, metadata, and media) easy to manage and ship.

---

## 3. Target Users
- Writers and podcasters
- Researchers and academics
- Knowledge management teams

---

## 4. Core Concepts
- **Project**: A folder following cite-cli’s standardized project structure.
- **Manifest**: `cite.toml` at the root — central configuration for project metadata, build settings, and backend targets.
- **Content files**: Pure Markdown (`.md`), BibTeX (`.bib`) for citation, reStructuredText (`.rst`), or other supported text formats with **no embedded frontmatter** — metadata is kept separate.
- **Metadata file**: A single `metadata.yml` at the project root that describes all content (artists, news, podcasts) with references to content files.
- **Slug**: The canonical, kebab-case identifier used across metadata, file references, and database records. Must be unique per content type.
- **Compiler Protocol**: A versioned process that transforms source files into a structured output bundle ready for database ingestion.
- **Staging Environment**: All deployments target the staging environment/schema exclusively, ensuring a safe, isolated testing ground.

---

## 5. Key Commands

| Command              | Description                                              |
|----------------------|----------------------------------------------------------|
| `cite-cli init <name>` | Create a new project with recommended structure and starter files. |
| `cite-cli validate`  | Run full validation (structure, files, metadata completeness, cross-references). |
| `cite-cli lint`      | Run linting rules (naming conventions, broken links, audio metadata, style, word counts). |
| `cite-cli build`     | Execute the compiler protocol and produce a build artifact. |
| `cite-cli deploy`    | Deploy the built project to the configured Supabase staging target. |
| `cite-cli status`    | Show project health, validation summary, and sync state. |
| `cite-cli doctor`    | Diagnose common project issues and configuration problems. |
| `cite-cli clean`     | Remove build artifacts, temporary files, and build cache. |

---

## 6. Detailed Functional Requirements

### 6.1 Project Scaffolding (`init`)
Creates the standard folder structure:

```bash
<project-name>/
├── cite.toml                  # Project manifest
├── metadata.yml               # Single YAML file describing all content metadata
├── content/                   # Raw content files (pure .md, .rst, .bib optional)
├── assets/
│   ├── audio/                 # Audio files (mp3, wav, m4a)
│   └── images/                # Images, thumbnails, cover art
└── build/                     # Output directory (gitignored)
```

Generates a starter `cite.toml` with sensible defaults for:
- Project name, version, authors
- Build and compiler settings
- Supabase connection details for the staging environment
- Metadata format preference

### 6.2 Content & Metadata Model
Content files are **pure text** (no embedded frontmatter). All metadata lives in a single `metadata.yml` at the project root.

Each content file in `content/` is referenced by its `slug` in `metadata.yml`.

**Example `metadata.yml`**:

```yaml
artists:
  - slug: jane-doe
    name: "Jane Doe"
    email: jane.doe@example.com

news:
  - slug: my-article-1
    title: "My Article Title"
    file: content/my-article.md
    citation: content/my-article.bib
    category: "artificial intelligence"
    artists: [jane-doe] # Supports multiple authors
    podcasts: [my-podcast-1]

podcasts:
  - slug: my-podcast-1
    title: "My Podcast Episode"
    file: assets/audio/podcast.mp3
    duration_seconds: 2700

```

All relational references (`podcasts`, `artists`, etc.) use the target item’s `slug` and are defined as arrays to support one-to-many relationships.

### 6.3 Validation & Linting
- Validate project structure against `cite.toml` expectations.
- Parse and validate `metadata.yml` structure and required sections.
- Validate metadata fields against required schema per type.
- Check that every referenced content file and asset exists on disk.
- Cross-reference validation: internal links, slug references resolve correctly.
- Audio file validation: format, duration match, file existence.
- Naming convention checks (kebab-case slugs, consistency).
- Slug uniqueness validation per content type.
- Output clear, colored, hierarchical error/warning reports.

### 6.4 Compiler Protocol (`build`) — v0
Versioned protocol starting at v0. Phases:

1. **Parse & load** — read all content files and `metadata.yml`.
2. **Resolve references** — validate slugs and asset paths.
3. **Transform** — compile content to standardized Markdown, resolve cross-references, normalize dates/durations/paths, embed minimal or expanded metadata as configured.
4. **Validate completeness** — ensure required fields per type are present.
5. **Generate output bundle** — produce structured JSON file having a single `content.json` file with all items embedded from `metadata.yml`:

```bash
build/
├── content.json
└── assets/
```

- **Incremental builds**: The CLI maintains a hidden `.cite-cache.json` file storing file hashes to determine what has changed, only re-processing modified files.
- Configurable Markdown extensions (tables, footnotes, etc.).

### 6.5 Backend Integration (`deploy`)
- Authenticate with Supabase using the staging service role key from `cite.toml` (or environment variables).
- Deploy **exclusively** to the staging backend.
- **Idempotency**: Push content and metadata using `UPSERT` (e.g., `ON CONFLICT DO UPDATE`) to prevent duplicate rows or constraint violations on repeated runs.
- Upload assets to Supabase Storage using predictable paths (e.g., `/{project-slug}/{content-type}/{slug}/{filename}`) and rewrite URLs in payloads.
- Support selective deploy (changed items only, based on build manifest).
- Dry-run mode (`--dry-run`).
- **Rollback support**: Tag all deployed database rows with a unique `deployment_id`. Rollbacks are executed by deleting all records associated with a specific failed `deployment_id`.

### 6.6 Configuration (`cite.toml`)

```toml
[project]
name = "my-project"
version = "0.1.0"
default_language = "en"
metadata_file = "metadata.yml"

[build]
compiler_version = "0"
incremental = true
output_format = "json"

[compiler]
enabled_extensions = ["tables", "footnotes"]

[backend]
staging_url = "https://xxxxx.supabase.co"
staging_service_key = "..."     # prefer environment variable override (CITE_STAGING_SERVICE_KEY)

[assets]
audio_formats = ["mp3", "wav", "m4a"]
image_formats = ["jpg", "png", "webp"]

[validation]
strict = true
```

---

## 7. Non-Functional Requirements
- Single static binary, cross-platform (Linux, macOS, Windows).
- Fast execution and low resource usage.
- Clear, consistent, user-friendly terminal output with progress indicators and colored output.
- Excellent error messages with recovery suggestions.
- Secure credential handling: support environment variable overrides (`CITE_STAGING_SERVICE_KEY`). Service keys should **never** be committed to git or logged to the terminal.
- Extensible design — easy to add new content types, metadata schemas, and output formats.
- Comprehensive help (`cite-cli --help`, `cite-cli <command> --help`).
- Graceful handling of partial failures during deploy (using Postgres transactions to ensure atomic per-table commits).
- Git-friendly: `build/` and `.cite-cache.json` are gitignored by default.
- Structured observability using `tracing` with level-based filtering and opt-in verbose mode (`--verbose`).

---

## 8. High-Level Design Principles
- **Declarative**: Behavior driven by `cite.toml` — no hidden conventions.
- **Fail-fast & Transparent**: Validation and build give maximum visibility into issues.
- **Modular**: Each major feature (scaffold, lint, build, deploy) is internally independent.
- **Versioned Protocol**: Compiler protocol is versioned — breaking changes increment the version.
- **Observability**: Rich logging, summary reports, and detailed error output.
- **Staging-only**: All deployments target the staging environment exclusively, ensuring a safe, isolated testing ground.
- **Metadata/content separation**: Content is pure text; metadata is in separate manifests — keeps content portable and reusable.

---

The CLI operates as a stateless, local-first tool that reads a project directory, processes it through a versioned compiler protocol, and optionally pushes the resulting artifact to a remote Supabase staging environment.

**High-Level Flow:**
`Project Dir` → `Load Context` → `Validate/Lint` → `Incremental Build` → `Deploy`

## Project Structure
```text
cite-cli/
├── Cargo.toml                 # Dependencies (clap, serde, tokio, reqwest, sha2, tracing, etc.)
├── src/
│   ├── main.rs                # Entrypoint, tracing init, CLI dispatch
│   ├── cli.rs                 # Clap command definitions
│   ├── manifest.rs            # cite.toml parsing & types
│   ├── metadata.rs            # metadata.yml parsing & types
│   ├── slug.rs                # Kebab-case Slug newtype & validation
│   ├── project.rs             # Project loading & context aggregation
│   ├── validation.rs          # Validation & linting engine
│   ├── compiler.rs            # Compiler protocol (build)
│   ├── deploy.rs              # Supabase staging deploy & rollback logic
│   ├── cache.rs               # Incremental build cache (SHA-256)
│   ├── scaffold.rs            # init & clean commands
│   └── error.rs               # Centralized, user-friendly error types
└── tests/
    └── integration.rs         # High-level CLI integration tests
```
