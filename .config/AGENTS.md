# Tool configuration

Tool configuration lives in this directory, not at the repository root,
mirroring the `~/.config` home that the
[XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/latest/)
defines.

## Naming

- Name a single-file config `<tool>.<ext>`, for example `typos.toml`.
- Name the file after the tool, so two tools never compete for a generic name.
- Drop the leading dot and any `.config` or `rc` suffix the tool uses at the
  root: `.taplo.toml` becomes `taplo.toml`.
- Keep the file extension and use lowercase names.
- Use a `<tool>/` directory when the tool needs more than one file, as `mise/`
  and `buf/` do.

## Wiring a tool

Most tools only discover their config at the repository root. When a tool does
not look here on its own, pass the path explicitly in the mise task under
`mise/tasks/` that runs it, so CI and local runs read the same file. When a
workflow filters on the config path, update its `paths:` to the new location.

## What stays at the root

These stay at the root: `.gitignore`, `README.md`, `LICENSE`, `SECURITY.md`,
`AGENTS.md` and `mise.toml`. GitHub-specific configuration belongs in
`.github/`. A package's own config stays next to that package when its
tooling resolves it from the package directory.
