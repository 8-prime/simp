# Installer GUI prototype

The end-user installer window uses GPUI Kit. The UI lives in `src/ui`; the
package reader lives in `src/install`. Installation operations and Windows target
resolution will live outside the UI. This prototype does not install files yet.

## Try it

From the workspace root:

```powershell
# Ready -> Installing -> Finished (click Install)
cargo run -p simp-installer --features ui-preview

# Ready -> Installing -> Failed (click Install)
cargo run -p simp-installer --features ui-preview -- --preview-failure
```

The preview takes about five seconds, uses a predefined manifest, and never writes
application files. It does not need a package appended to the executable. The
failure scenario stops partway through. Close works at
any point; Restart preview returns to Ready after either outcome.

The Cargo feature selects preview mode by default. The `--preview` option also
selects the successful preview; `--preview-failure` selects the failure scenario.
Use `--runtime` to read the appended manifest from an executable built with the
preview feature.
Normal launches read the appended manifest from their own executable and display
its application details. Installation is still under development. A normal
unpackaged `cargo run` launch reports the missing package in the window.
Unknown arguments show an error in the window.

```powershell
cargo run -p simp-installer
cargo build -p simp-installer --release
```

Release builds hide the console window on Windows. Keep the `ui-preview` feature
out of distributable installer builds. Rebuild `simp-cli` after rebuilding the
release installer: it currently embeds that executable at compile time.

## Development

Use the Rust MSVC toolchain and Visual Studio C++ build tools with a Windows SDK.
GPUI's release build also needs the SDK's `fxc.exe` shader compiler (normally
discovered automatically; `GPUI_FXC_PATH` can point to it explicitly).

```powershell
cargo fmt -p simp-installer -- --check
cargo test -p simp-installer
cargo test -p simp-installer --features ui-preview
```

Before shipping, check the release executable on clean Windows and Remote
Desktop, at 100%, 150%, and 200% display scaling. Check Tab / Shift+Tab navigation,
Enter / Space activation, progress responsiveness, both outcomes, window closing,
and whether the app exits after closing its last window. Measure the release
executable size and verify its runtime dependencies on that clean machine.

## Next integration

Validate the appended package footer and run a synchronous installation operation on a worker, with
an event sink forwarding semantic progress to the UI. Keep GPUI, channels, and
thread scheduling in the UI adapter. Add real cancellation/close handling when
installation side effects exist; the current Close behavior only stops a preview.
