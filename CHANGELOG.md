# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1](https://github.com/Agent-Card/ai-catalog-cli/compare/v0.3.0...v0.3.1) - 2026-10-21

### Other

- update Homebrew formula for v0.3.0 ([#56](https://github.com/Agent-Card/ai-catalog-cli/pull/56))

## [0.3.0](https://github.com/Agent-Card/ai-catalog-cli/compare/v0.2.2...v0.3.0) - 2026-10-21

### Added

- [**breaking**] adopt ai-catalog 0.3 and report diagnostic codes ([#51](https://github.com/Agent-Card/ai-catalog-cli/pull/51))
- add openssf scoreboard to the repository ([#18](https://github.com/Agent-Card/ai-catalog-cli/pull/18))

### Fixed

- [**breaking**] enforce the specs fetching rules and verify subject.digest on pull ([#52](https://github.com/Agent-Card/ai-catalog-cli/pull/52))
- de-duplicate identifiers in emitted catalogs ([#44](https://github.com/Agent-Card/ai-catalog-cli/pull/44))
- add homebrew ([#13](https://github.com/Agent-Card/ai-catalog-cli/pull/13))

### Other

- rewrite local storage docs ([#54](https://github.com/Agent-Card/ai-catalog-cli/pull/54))
- [**breaking**] parse the command line with clap ([#53](https://github.com/Agent-Card/ai-catalog-cli/pull/53))
- *(deps)* update rust to v1.2.0 ([#41](https://github.com/Agent-Card/ai-catalog-cli/pull/41))
- *(deps)* update dependency renovatebot/renovate to v44.151.4 ([#40](https://github.com/Agent-Card/ai-catalog-cli/pull/40))
- *(deps)* update github actions ([#43](https://github.com/Agent-Card/ai-catalog-cli/pull/43))
- [**breaking**] remove trust inspect in favour of validate ([#47](https://github.com/Agent-Card/ai-catalog-cli/pull/47))
- document WinGet installation in README ([#46](https://github.com/Agent-Card/ai-catalog-cli/pull/46))
- *(deps)* update github actions ([#42](https://github.com/Agent-Card/ai-catalog-cli/pull/42))
- *(deps)* update dependency renovatebot/renovate to v44.126.1 ([#39](https://github.com/Agent-Card/ai-catalog-cli/pull/39))
- *(deps)* update rust ([#36](https://github.com/Agent-Card/ai-catalog-cli/pull/36))
- *(deps)* update dependency renovatebot/renovate to v44.124.2 ([#38](https://github.com/Agent-Card/ai-catalog-cli/pull/38))
- *(deps)* update dependency renovatebot/renovate to v44.121.4 ([#37](https://github.com/Agent-Card/ai-catalog-cli/pull/37))
- *(deps)* update dependency renovatebot/renovate to v44.115.13 ([#35](https://github.com/Agent-Card/ai-catalog-cli/pull/35))
- *(deps)* update rust ([#28](https://github.com/Agent-Card/ai-catalog-cli/pull/28))
- *(deps)* update dependency renovatebot/renovate to v44.115.4 ([#34](https://github.com/Agent-Card/ai-catalog-cli/pull/34))
- *(deps)* update rust ([#32](https://github.com/Agent-Card/ai-catalog-cli/pull/32))
- *(deps)* update dependency renovatebot/renovate to v44.115.2 ([#29](https://github.com/Agent-Card/ai-catalog-cli/pull/29))
- *(deps)* update actions-rust-lang/setup-rust-toolchain action to v2 ([#31](https://github.com/Agent-Card/ai-catalog-cli/pull/31))
- change renovate config ([#33](https://github.com/Agent-Card/ai-catalog-cli/pull/33))
- *(deps)* update github actions ([#30](https://github.com/Agent-Card/ai-catalog-cli/pull/30))
- add CI success gate ([#25](https://github.com/Agent-Card/ai-catalog-cli/pull/25))
- add OpenSSF scorecard improvements ([#24](https://github.com/Agent-Card/ai-catalog-cli/pull/24))
- add PRIVACY.md ([#23](https://github.com/Agent-Card/ai-catalog-cli/pull/23))
- add issue templates ([#22](https://github.com/Agent-Card/ai-catalog-cli/pull/22))
- add CONTRIBUTORS.md file ([#19](https://github.com/Agent-Card/ai-catalog-cli/pull/19))
- render and validate WinGet manifests on release ([#16](https://github.com/Agent-Card/ai-catalog-cli/pull/16))
- update Homebrew formula for v0.2.2 ([#15](https://github.com/Agent-Card/ai-catalog-cli/pull/15))

## [0.2.2](https://github.com/Agent-Card/ai-catalog-cli/compare/v0.2.1...v0.2.2) - 2026-09-01

### Other

- use draft release plz ([#11](https://github.com/Agent-Card/ai-catalog-cli/pull/11))

## [0.2.1](https://github.com/Agent-Card/ai-catalog-cli/compare/v0.2.0...v0.2.1) - 2026-09-01

### Fixed

- *(release)* use trusted publising ([#9](https://github.com/Agent-Card/ai-catalog-cli/pull/9))

### Other

- remove AGNTCY references, migrate to trusted publishing ([#8](https://github.com/Agent-Card/ai-catalog-cli/pull/8))

## [0.2.0](https://github.com/Agent-Card/ai-catalog-cli/compare/v0.1.0...v0.2.0) - 2026-08-14

### Added

- adopt SDK 0.2.0 and rework the consumer demo ([#7](https://github.com/Agent-Card/ai-catalog-cli/pull/7))

### Fixed

- example catalog, bare path support, inline data pull/show, and refactor ([#6](https://github.com/Agent-Card/ai-catalog-cli/pull/6))

### Other

- add --media-type, URI-based scope, SKILL.md ([#4](https://github.com/Agent-Card/ai-catalog-cli/pull/4))

## [0.1.0](https://github.com/Agent-Card/ai-catalog-cli/releases/tag/v0.1.0) - 2026-08-07

### Added

- import the ai-catalog CLI from ai-catalog-rust ([#1](https://github.com/Agent-Card/ai-catalog-cli/pull/1))

### Other

- attach prebuilt binaries to releases ([#3](https://github.com/Agent-Card/ai-catalog-cli/pull/3))
- add license, code owners, and gitignore
