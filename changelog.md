# Unreleased (changes since v0.8.3)

This changelog entry contains the changes merged to `main` after the v0.8.3 bump and should be embedded into release notes by the release workflow.

## Added

- Add PUFFDIFF install-operation support (apply PUF1 patches via the `puffdiff` crate) — merged in PR #14 and implemented in diff.rs. ([merge commit](https://github.com/rhythmcache/payload-dumper-rust/commit/79e3e9326d37b1144fd80f076690eb505401aa07), implementation commit: https://github.com/rhythmcache/payload-dumper-rust/commit/04f6b067e449cae435e9f160dec79fb6da558649)
- Include `source` and `zip` information in the metadata JSON output. ([commit](https://github.com/rhythmcache/payload-dumper-rust/commit/21687295f393819cec1e27f2f66ded794d85436d))

## Changed

- Improve metadata output formatting and fields. ([commit](https://github.com/rhythmcache/payload-dumper-rust/commit/5fa31dd6084187961896d08d850e8679e7b5e72a))
- Add a CLI option to specify a custom DNS server and document its usage in the README. ([cli option commit](https://github.com/rhythmcache/payload-dumper-rust/commit/fa86c9469e1dfa3fd632d1611eb062b6b2a32db2), [docs commit](https://github.com/rhythmcache/payload-dumper-rust/commit/e571076d68a0dae70aeeb4e9fe3cb2778d588221))
- Change default thread count to use CPU count (fix incorrect thread-count behavior). ([commit](https://github.com/rhythmcache/payload-dumper-rust/commit/f35cc041119fb3270cc1f8cfc2f62256f0e95c98))

## Fixed

- Fix wrong patch method handling. ([commit](https://github.com/rhythmcache/payload-dumper-rust/commit/7507dfebc4a7d6a563825a46f2f6ef0832d6a63b))
- Various error-message and argument-description fixes (improved UX and clearer errors). ([example commits](https://github.com/rhythmcache/payload-dumper-rust/commit/3c349bad359833ed5e1abd72da885458b1ea739f), https://github.com/rhythmcache/payload-dumper-rust/commit/7a9b4eac7cfee31c4617e71353dfe6b78cdd20ec)
- Update hickory proto and resolver. ([commit](https://github.com/rhythmcache/payload-dumper-rust/commit/0dbc072ef423e7ff9e828a9e7cc96b43e3b8b2c4))

## Performance

- Increased buffer sizes and optimized zero handling by leveraging sparse file writes for higher throughput. ([commit](https://github.com/rhythmcache/payload-dumper-rust/commit/aa01b07d1876cffa04dc347d15f113030833406f))

## Build / Dependencies

- Multiple dependency bumps and general dependency maintenance across the tree. (See commits with "bump dep"/"updated dep" in history.)
- Removed `rustls` from dependencies and adjusted `reqwest` usage/features (feature removals and later refactorings). ([remove rustls commit](https://github.com/rhythmcache/payload-dumper-rust/commit/b6037c9ac8b9d65b3492b2a5535bb7a51fa86c97), [remove reqwest feature commit](https://github.com/rhythmcache/payload-dumper-rust/commit/5dcc2750f1a93a831616d070b405fe0d3ed859d0))
- Update build script / include build year in build environment. ([commit](https://github.com/rhythmcache/payload-dumper-rust/commit/d5043fd7c4432111efae6a1c7b89ffff752c4ebc), [related commit](https://github.com/rhythmcache/payload-dumper-rust/commit/c7b735d85831b852354c67d71d5449569b117d56))

## Misc / Housekeeping

- Corrected Cargo.toml and fixed small packaging issues. ([commit](https://github.com/rhythmcache/payload-dumper-rust/commit/fb91c1e5b897ec8909926099db61799995e57093))
- Several small fixes and maintenance commits (dependency bumps, small text fixes, and CI/build improvements).

---

If you want this to include every single commit message and link (exhaustive), or to be organized with dates and author names for each entry, I can expand it. 
