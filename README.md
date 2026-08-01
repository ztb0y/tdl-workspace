# tdl-workspace

Shared source-mode workspace for the official `tdl`, the `tdl-web` extension,
and its separate `tdl-web-ui` frontend.

## Clone

```sh
git clone --recurse-submodules https://github.com/ztb0y/tdl-workspace.git
```

For an existing checkout:

```sh
git submodule update --init --recursive
```

## Modes

`go.work` resolves the extension's `tdl`, `core`, and `extension` imports to
the local official source while working in this repository. The standalone
`tdl-web` repository keeps using its GitHub module dependencies with
`GOWORK=off` for CI and releases.

The workspace pins `tdl` to official `v0.20.3`. Test unmerged `tdl` changes
only in a local branch inside `tdl/`; do not commit that submodule pointer
until the corresponding upstream change is available.

## Windows

Install Go 1.25.8 or later and Node.js 22 or later, then double-click
`Start-Workspace.cmd`.

The launcher builds `tdl.exe` and `tdl-web.exe`, refreshes the local `web`
extension with `tdl extension install --force`, starts `tdl web` and Vite, and
opens <http://localhost:5173>. Press `Ctrl+C` or close the launcher window to
stop both processes. Account data under `.tdl/extensions/data/web` is kept.

## Frontend

The UI is an independent repository in `tdl-web-ui`. It remains separately
deployable and uses `VITE_API_BASE` to target the extension API.
