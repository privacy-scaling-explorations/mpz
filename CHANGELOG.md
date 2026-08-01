# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- `Context::map` no longer opens one multiplexer channel per item. Items are distributed
  round-robin over at most `concurrency_limit` lanes, so the number of channels a `map` opens
  is bounded by the limit rather than by the size of the workload.
