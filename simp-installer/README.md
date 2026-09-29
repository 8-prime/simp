# Installer GUI prototype

The end-user installer window uses GPUI Kit. The UI lives in `src/ui`; the
package reader lives in `src/install`. Installation operations and Windows target
resolution will live outside the UI. This prototype does not install files yet.

## Try it

From the workspace root:

```powershell
# Welcome -> License -> Location -> Tasks -> Ready -> Installing -> Finished
cargo run -p simp-installer --features ui-preview

# The same wizard, with a simulated failure during installation
cargo run -p simp-installer --features ui-preview -- --preview-failure
```

The wizard follows the seven setup screens in the design reference. It uses
Segoe UI, white content panels, blue primary buttons, step dots, a summary panel,
and a green progress bar with expandable details. The native title bar follows
the Windows appearance setting.

The preview uses a predefined manifest and never writes application files or
changes system settings. Accept the sample license to enable Next, enter an
absolute installation path or use Browse, and select optional tasks. Back keeps
your choices; the summary shows the current path and selected tasks. License and
task pages are omitted when those values are absent from a manifest.

Click Install on the summary to run the five-second simulation. Cancel closes
the window, including during progress. The failure scenario stops partway through;
Try again returns to the summary. Finish closes the successful preview. The launch
and release-note checkboxes demonstrate the completion screen only; they do not
launch anything. The preview needs no package appended to the executable.

The Cargo feature selects preview mode by default. The `--preview` option also
selects the successful preview; `--preview-failure` selects the failure scenario.
Use `--runtime` to read the appended manifest from an executable built with the
preview feature.
Normal launches read the appended manifest from their own executable and show
the same wizard with the application's metadata. Install stays disabled until the
installation operation is implemented. A normal unpackaged `cargo run` launch
reports the missing package in the window.
Unknown arguments show an error in the window.

```powershell
cargo run -p simp-installer
cargo build -p simp-installer --release
```

Release builds hide the console window on Windows. Keep the `ui-preview` feature
out of distributable installer builds. Rebuild `simp-cli` after rebuilding the
release installer: it currently embeds that executable at compile time.

The Windows application icon is generated from `../assets/simpicon.png` as
`assets/simp-installer.ico` with 16, 24, 32, 48, 64, 128, and 256 pixel images.
The build script embeds it as icon resource 1, which GPUI also uses for the
installer window. Rebuild the release installer before rebuilding `simp-cli` to
include icon changes in packaged installers.

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
