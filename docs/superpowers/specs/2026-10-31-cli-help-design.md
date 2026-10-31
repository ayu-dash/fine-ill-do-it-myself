# CLI Help Alignment Design

## Intent

Make `fidim --help` and subcommand help accurately describe the commands and
features that already exist, so users can discover the current CLI without
reading source code or relying on stale wording.

## Goals

- Add concise descriptions to currently undocumented top-level commands.
- Document supported web servers consistently: `caddy`, `httpd`/`apache`,
  `nginx`, and `lighttpd`.
- Describe installable managed components and the `new` project templates.
- Make nested help useful for `web`, `dns`, `php`, language runtimes, and
  `ports`.
- Add regression tests that assert important help text and supported choices.

## Non-goals

- No new CLI commands or behavior.
- No changes to command parsing, aliases, defaults, or installation logic.
- No replacement of the README as full product documentation.

## Approaches

1. **Minimal text edits** — update only stale descriptions. This is low risk
   but leaves several commands and nested options undocumented.
2. **Consistent help pass** — revise all existing clap descriptions and add
   focused examples where discovery matters, while keeping parsing unchanged.
   This is the selected approach because the mismatch spans the command tree.
3. **Generated external CLI reference** — move help content into a separate
   documentation generator. This adds maintenance and tooling complexity without
   changing the underlying issue.

## Design

The implementation will stay in `crates/fidim/src/main.rs`, using clap
attributes only. Top-level commands will receive short descriptions. The
`web` help will name all four supported server identifiers and explain that
`apache` is accepted as the user-facing alias for `httpd` if the parser
supports it; otherwise the wording will use the actual accepted identifier.
Install help will list representative managed names, including Apache,
Nginx, lighttpd, PHP-FPM, MariaDB, Composer, and phpMyAdmin where supported.

Nested subcommands will retain their current arguments and add concise
descriptions for actions and values. Tests will invoke the built binary with
`--help` and selected nested help commands using `assert_cmd`, asserting
stable feature names rather than the full formatted output.

## Acceptance criteria

- Help output no longer claims an incomplete web-server list.
- Every existing top-level command has a meaningful description.
- Important nested commands expose their purpose and supported values.
- CLI help regression tests pass without network, filesystem side effects, or
  dependence on terminal width.
- Existing workspace tests and clippy remain clean.
