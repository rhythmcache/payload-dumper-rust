# Changes in v0.8.4

## Added
- PUFFDIFF install-operation support: apply PUF1 patches via the `puffdiff` crate (PR #14).
- Include `source` and `zip` information in metadata JSON output.

## Changed
- Improve metadata output formatting and fields.
- Add CLI option to specify a custom DNS server and document its usage.
- Change default thread count to use the system CPU count.

## Fixed
- Fix wrong patch method handling.
- Improve error messages and CLI argument descriptions.
- Update hickory protocol and resolver code.

## Performance
- Increased buffer sizes and optimized zero-handling using sparse file writes for better throughput.

## Build & Dependencies
- Dependency bumps and maintenance across the tree.
- Removed `rustls` and adjusted `reqwest` usage/features.
- Updated build script to include the build year in environment variables.

## Misc
- Corrected Cargo.toml and other packaging fixes.
