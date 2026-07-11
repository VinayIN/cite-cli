# cite-cli — Product Requirements Document

## 1. Product Overview
**cite-cli** is a CLI tool for creating, structuring, validating, building, and deploying podcast content to a Supabase backend consumable by the aouxAI application.

## 2. Objectives
- Accelerate project initialization with consistent scaffolding.
- Ensure content quality through automated validation.
- Provide reliable build with incremental caching.
- Enable safe deployment with full-data backup and rollback.

## 3. Core Concepts
- **Project**: A folder with `cite.toml`, `metadata.yml`, and content directories.
- **Artist**: A pre-existing Supabase user identified by UUID in `metadata.yml`. The CLI user is the content creator.
- **Podcast**: A content item consisting of a markdown file, optional audio, optional source URL, optional thumbnail, and optional BibTeX citation.
- **Compiler Protocol**: Versioned process transforming source files into a `content.json` bundle.
- **Deployment**: Full `content.json` uploaded to Supabase Storage as `{deployment_id}.json`, plus a subset written to database tables for queryability.

## 4. Metadata Model (`metadata.yml`)

```yaml
artist_id: "11111111-1111-1111-1111-111111111111"
podcasts:
  - title: "My Podcast"
    file: content/my-article.md
    source_url: "https://example.com"
    category: "artificial intelligence"
    audio: assets/audio/episode.mp3
    thumbnail: assets/images/thumb.jpg
    citation: content/my-article.bib
```

- `artist_id` references an existing artist record in the database.
- `podcasts` is an array of content items. Each has a title, markdown file, and optional audio/thumbnail/citation.
- No slugs, no junction tables, no tier-based audio variants.

## 5. Key Commands

| Command | Description |
|---------|-------------|
| `init <name>` | Create project structure with starter files |
| `validate` | Check structure, metadata, file existence, asset formats |
| `lint` | Word count and content quality checks |
| `build` | Compile content → `build/content.json` with UUIDs and BibTeX timelines |
| `deploy` | Upload full bundle to storage; populate `news`, `podcasts`, `artists_news`, `timeline` tables |
| `status` | Project health overview |
| `doctor` | Diagnose configuration and dependency issues |
| `clean` | Remove `build/` directory and cache |
| `rollback <id>` | Delete rows tagged with deployment_id from `podcasts`, `artists_news`, `news` |

## 6. Build Pipeline

```
Project Dir → Load Context → Validate/Lint → Incremental Build → content.json
```

The compiler:
- Assigns UUIDv4 to each podcast item
- Reads and embeds markdown content
- Parses BibTeX citations into timeline entries
- Rewrites asset paths relative to build directory
- Maintains SHA-256 incremental cache

## 7. Deploy Pipeline

```
content.json → Upload full bundle to storage as {deployment_id}.json
            → For each podcast:
                Resolve category by name
                Create url record from source_url
                Insert news row
                Auto-create artists_news junction
                Upload audio → insert podcasts row
                Parse BibTeX timelines → insert timeline + timeline_news
```

- All rows tagged with `deployment_id` for rollback.
- Dry-run mode (`--dry-run`) shows what would happen without writing.
- Service key sourced from env var `CITE_STAGING_SERVICE_KEY` or `cite.toml`.

## 8. Project Structure

```
my-project/
├── cite.toml              # Project manifest
├── metadata.yml           # Artist ID + podcast list
├── content/               # Markdown content files
├── assets/
│   ├── audio/             # MP3/WAV/M4A files
│   └── images/            # Thumbnails, cover art
└── build/                 # Output (gitignored)
```

## 9. Design Principles
- **No slugs** — content identified by UUID, artist by database-assigned UUID.
- **No junctions in metadata** — all relations handled automatically at deploy time.
- **No tiers** — single audio file per podcast if present.
- **Metadata/content separation** — content files are pure text; metadata is separate.
- **Full backup** — entire bundle preserved in storage for recovery.
- **Staging-only** — all deployments target staging via service key.
