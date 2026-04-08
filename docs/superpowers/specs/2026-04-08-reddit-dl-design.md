# reddit-dl: Bulk Reddit Downloader (Rust)

## Context

Existing Python Reddit bulk downloaders (BDFR, saveddit, etc.) lack first-class support for downloading from a user's friends list and followed accounts. This project builds a Rust CLI tool that:

- Downloads from **all sources**: friends, follows, saved posts, subreddits, and arbitrary users
- Archives **everything**: media, metadata (JSON), text/self posts, and optionally comments
- **Tracks progress** with a SQLite database so re-runs only fetch new content
- **Deduplicates** across sources to avoid redundant downloads

## Architecture: Direct HTTP Client

Built on `reqwest` + `serde` with hand-written Reddit API types. No wrapper crate (Roux) — this gives full control over friends/follows endpoints that wrapper crates don't cover.

### Crate Dependencies

| Crate | Purpose |
|-------|---------|
| `reqwest` | Async HTTP client |
| `tokio` | Async runtime |
| `serde` / `serde_json` | Serialization |
| `clap` (derive) | CLI argument parsing |
| `rusqlite` | SQLite for dedup DB + cursors |
| `toml` | Config file parsing |
| `indicatif` | Progress bars |
| `tracing` / `tracing-subscriber` | Structured logging |
| `directories` | Platform-appropriate config/data paths |
| `chrono` | Timestamp handling |

### Project Structure

```
reddit-dl/
├── Cargo.toml
├── config.example.toml
├── src/
│   ├── main.rs                # CLI entry point (clap)
│   ├── config.rs              # TOML config + CLI merge
│   ├── api/
│   │   ├── mod.rs
│   │   ├── auth.rs            # OAuth2 flow + token refresh
│   │   ├── client.rs          # Rate-limited reqwest client
│   │   ├── types.rs           # Reddit API response types
│   │   └── endpoints.rs       # Friends, follows, saved, subreddit, user
│   ├── sources/
│   │   ├── mod.rs
│   │   ├── friends.rs         # Fetch friends list -> iterate their posts
│   │   ├── follows.rs         # Fetch followed users -> iterate their posts
│   │   ├── saved.rs           # Fetch saved posts
│   │   ├── subreddit.rs       # Fetch subreddit posts
│   │   └── user.rs            # Fetch arbitrary user's posts
│   ├── download/
│   │   ├── mod.rs
│   │   ├── media.rs           # Image/video/gallery downloading
│   │   ├── text.rs            # Self-post / comment archiving
│   │   └── resolver.rs        # URL resolution (imgur, reddit galleries, etc.)
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── db.rs              # SQLite dedup database + cursors
│   │   ├── filesystem.rs      # Output directory structure
│   │   └── metadata.rs        # JSON metadata writer
│   └── progress.rs            # Progress bars / reporting
```

### Data Flow

```
Config -> Source(s) -> API Client -> Post Iterator -> Dedup Check -> Downloader -> Filesystem + DB Update
```

## SQLite Database

Located at `{output_directory}/reddit-dl.db`.

### Schema

```sql
CREATE TABLE posts (
    id TEXT PRIMARY KEY,              -- Reddit post fullname (e.g., "t3_abc123")
    source_type TEXT NOT NULL,        -- "friends", "follows", "saved", "subreddit", "user"
    source_name TEXT NOT NULL,        -- username, subreddit name, or "saved"
    title TEXT,
    author TEXT,
    permalink TEXT,
    created_utc INTEGER,
    downloaded_at INTEGER NOT NULL,
    media_count INTEGER DEFAULT 0
);

CREATE TABLE cursors (
    source_type TEXT NOT NULL,
    source_name TEXT NOT NULL,
    last_post_id TEXT NOT NULL,
    last_post_utc INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (source_type, source_name)
);
```

### Resume Logic

1. On sync, look up cursor for each source in `cursors` table
2. Fetch posts from Reddit API newest-first
3. Stop when hitting the cursor's `last_post_id` or `last_post_utc`
4. Check each post against `posts` table for cross-source dedup
5. After successful download batch, update cursor
6. `--full` flag ignores cursors but still deduplicates via `posts` table

## Configuration

### Config File (`config.toml`)

```toml
[auth]
client_id = "your_client_id"
client_secret = "your_client_secret"
username = "your_username"
password = "your_password"

[output]
directory = "~/reddit-archive"
create_subdirs = true

[download]
max_concurrent = 5
include_metadata = true
include_comments = false
file_naming = "id"             # "id", "title", or "id-title"

[sources]
friends = true
follows = true
saved = true
subreddits = ["pics", "earthporn", "wallpapers"]
users = ["specific_user1", "specific_user2"]
```

### CLI Commands

```
reddit-dl sync                          # sync all configured sources
reddit-dl sync --source friends         # sync only friends
reddit-dl sync --source subreddit:pics  # sync one subreddit
reddit-dl sync --full                   # ignore cursors, full re-download
reddit-dl status                        # show cursor positions and stats
reddit-dl auth                          # interactive OAuth setup
```

CLI flags override config file values. Config file location defaults to `~/.config/reddit-dl/config.toml` (XDG on Linux, platform-appropriate elsewhere via `directories` crate).

## Output Directory Structure

```
~/reddit-archive/
├── reddit-dl.db
├── friends/
│   └── {username}/
│       ├── {post_id}.jpg
│       ├── {post_id}.json
│       └── {post_id}_gallery/
│           ├── 1.jpg
│           └── 2.jpg
├── follows/
│   └── {username}/
│       └── ...
├── saved/
│   └── {subreddit}/
│       └── ...
├── subreddits/
│   └── {subreddit}/
│       └── ...
└── users/
    └── {username}/
        └── ...
```

## Download Pipeline

### Per-post pipeline

1. **Resolve URL** — determine media type
2. **Dedup check** — skip if post ID exists in `posts` table
3. **Download media** — async with bounded concurrency
4. **Write metadata** — JSON sidecar file
5. **Record in DB** — insert into `posts`, update cursor

### URL Resolution

| Source | Handling |
|--------|----------|
| `i.redd.it` | Direct image download |
| `v.redd.it` | Video + audio stream merging (Reddit splits them) |
| Reddit galleries | Fetch gallery metadata API, download each image |
| `imgur.com` | Single image and album support |
| Self-posts | Save as `.md` markdown file |
| External links | Save URL + metadata only (don't download arbitrary sites) |

### Reddit Video (v.redd.it) Merging

Reddit stores video and audio as separate DASH streams. The tool will:
1. Download video stream (highest available quality)
2. Download audio stream (if present)
3. Merge using `ffmpeg` if available on PATH, otherwise save video-only with a warning

## API & Rate Limiting

### Authentication

Reddit OAuth2 "script" flow (password grant for personal scripts):
1. POST to `https://www.reddit.com/api/v1/access_token` with client credentials + user credentials
2. Receive access token (1-hour expiry) + optional refresh token
3. All API calls use `https://oauth.reddit.com` with `Authorization: Bearer {token}`
4. Auto-refresh token before expiry

### Rate Limiting

- Reddit allows 60 requests/minute with OAuth
- Token bucket rate limiter in `api/client.rs`
- Exponential backoff on 429/5xx responses
- Custom `User-Agent` header (Reddit requires descriptive user agents)

### Key Endpoints

| Endpoint | Purpose |
|----------|---------|
| `GET /api/v1/me/friends` | List friends |
| `GET /subreddits/mine/subscriber` | List followed subreddits (follows proxy) |
| `GET /user/{username}/overview` | User's posts |
| `GET /user/{username}/saved` | Saved posts |
| `GET /r/{subreddit}/new` | Subreddit posts |
| `GET /api/info?id={fullname}` | Post details |
| `GET /comments/{article}` | Post comments |

**Note on "follows":** Reddit's follow system maps users to a special subreddit `u_{username}`. The friends list endpoint is well-defined. For followed users, we may need to scrape the subscribed subreddits and filter for `u_*` prefixed ones, or use the `/subreddits/mine/subscriber` endpoint.

## Error Handling

| Error Type | Strategy |
|------------|----------|
| Transient (network, 429, 5xx) | Retry with exponential backoff, 3 attempts |
| Permanent (404, 403) | Log warning, skip post, continue |
| Auth (401) | Refresh token, retry once |
| Disk full / IO errors | Log error, abort current source, continue others |

All errors logged via `tracing`. The tool never crashes the entire sync for one failed post.

## Concurrency Model

- `tokio` async runtime
- API calls: sequential per source (respect rate limits + maintain cursor order)
- Media downloads: parallel within a batch (bounded by `max_concurrent` semaphore from config)
- Multiple sources processed sequentially (share the same rate limit budget)

## Verification Plan

1. **Unit tests**: Config parsing, URL resolution, database operations
2. **Integration test**: Mock Reddit API responses, verify full pipeline
3. **Manual test**: Run against real Reddit account with `--limit 5` to verify OAuth, friends list, and download pipeline
4. **Resume test**: Run sync, then run again — verify no duplicates, cursor advances
5. **Status command**: Verify `reddit-dl status` shows correct cursor state
