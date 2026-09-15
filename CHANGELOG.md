# Changelog

All notable changes to embyr-server are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.1] - 2026-09-15

### Added
- Versioning/tagging/CHANGELOG release process (finding #19). `embyr-server`'s startup log now
  reports its own version, sourced from `CARGO_PKG_VERSION`.
- `docker-compose.yml` for local/single-host evaluation — starts `embyr-server` and Postgres 15
  together via the existing root `Dockerfile`, with dev-only default credentials and a named
  volume for data persistence.
- `docs/operations/release-process.md` documenting the version-bump/git-tag/CHANGELOG convention.
