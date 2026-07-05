# loc

A minimal backend counting lines of code for a public GitHub repo.

It downloads the repo's source as a tarball from GitHub, walks the files, and builds a directory
tree of line counts broken down by file extension.

## Run

```bash
cargo run
```

For dev mode with auto-reload on file changes (rebuilds and restarts the server whenever you save):

```bash
cargo install cargo-watch  # one-time
cargo dev
```

Listens on `http://0.0.0.0:3000` by default (override with the `PORT` env var). Optionally set
`GITHUB_TOKEN` to raise GitHub's API/rate limits and access private repos you have access to. Set
`RUST_LOG=debug` for more verbose logging (defaults to `info`).

## API

### `GET /:owner/:repo/locs`

Returns the full line-count tree.

Query params:
- `branch` — defaults to the repo's default branch.
- `filter` — comma-separated regexes matched against each file's extension key (e.g. `.ts$,.tsx$`)
  to only count matching files.

```json
{
  "loc": 42,
  "locByLangs": { ".rs": 40, "Dockerfile": 2 },
  "children": {
    "main.rs": 40,
    "Dockerfile": 2
  }
}
```

Folders are nested objects with the same shape; files are plain numbers (their line count).

### `GET /:owner/:repo/badge`

Same query params as above, plus `format=human` to abbreviate the count (e.g. `1.2k`). Returns a
[shields.io endpoint badge](https://shields.io/badges/endpoint-badge) payload:

```json
{ "schemaVersion": 1, "label": "lines", "message": "42", "cacheSeconds": 900 }
```

## Notes

This is intentionally minimal: no caching, no rate limiting, no repo size limits, and binary files
are skipped via a simple null-byte heuristic rather than full content-type detection.
