# Install

## macOS

Download `nomospice_x.y.z_aarch64.dmg` from the [releases](https://github.com/mprotogerakis/nomospice/releases), open it and drag nomospice into Applications. The app is signed and notarized, needs a Mac with Apple Silicon and macOS 11 or later, and includes ngspice.

## Windows

Download `nomospice_x.y.z_x64-setup.exe` (or the `.msi`) from the [releases](https://github.com/mprotogerakis/nomospice/releases) and run it. It includes ngspice. The installer is not code-signed yet: if SmartScreen warns, choose *More info → Run anyway*.

## Updates

From version 0.1.5 on, nomospice looks for a newer release when it starts. If there is one, the status bar shows *Update x.y.z*; click it to install. *Check for Updates…* in the app menu (macOS) or the Help menu (Windows) looks right away. Updates are signed and checked against a key built into the app.

## Linux and building from source

There are no Linux packages yet; nomospice builds from source on Linux, macOS and Windows. You need Rust, Node 20 or later, pnpm, and the ngspice shared library (`apt install libngspice0`, `brew install libngspice`):

```sh
git clone https://github.com/mprotogerakis/nomospice
cd nomospice
pnpm install
pnpm tauri dev
```

`pnpm tauri build` makes installers. nomospice looks for ngspice in `NGSPICE_LIB`, next to its executable, in the usual Homebrew and Linux paths, and through the system loader.

## Model libraries (optional)

nomospice comes with its own symbols and a few default models (1N4148, 2N3904, 2N3906, switches). For more, add a model library: see [Use model libraries](how-to/model-libraries.md).
