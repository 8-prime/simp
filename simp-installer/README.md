# Installer runtime and GUI

The end-user installer window uses GPUI Kit. The UI lives in `src/ui`; the
package reader and file installation operation live in `src/install`. The UI
adapter runs the operation on Tokio and forwards its events to GPUI. A packaged
runtime can install a flat set of files to the folder selected in the wizard.

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
the same wizard with the application's metadata. Install writes the package files
to the selected folder and reports completed-file counts, paths, and errors.
On failure, Try again returns to the location page and retains the package and
license consent. Already written files are not rolled back.
Cancel and native window closing are blocked while a real installation runs.
A normal unpackaged `cargo run` launch reports the missing package in the window.
On Windows, runtime task pages offer the configured shortcuts, autostart, and
PATH integration. Omitted tasks are hidden; `true` and `false` set their initial
checkbox state. Without a launch target, shortcut and autostart tasks are hidden.
`core_config.main_executable` identifies a packaged launch target relative to the
installation folder. `InstallOptions` carries one optional `main_executable`
path shared by desktop shortcuts, Start Menu entries, and autostart, with boolean
selections for each task. Without a launch target those three options are omitted;
adding the installation folder to PATH does not require a main executable.
Configured targets are validated when building and reading a package and before
installation writes files. Package format version 4 stores each file as an
independent zstd frame, followed by a bitcode metadata manifest and the footer.
Manifest entries contain absolute payload offsets and compressed/uncompressed
sizes; file contents are not serialized into the metadata. Rebuild the runtime
and CLI together before building new packages; earlier formats are rejected.
The CLI collects `SourceEntry` paths and streams one source file at a time through
async zstd compression into its buffered output writer. Runtime loading seeks to
the footer and reads only the `PackageManifest`. Installation streams each
compressed range into a destination file, checking frame completion, checksum,
and uncompressed size before reporting that file as completed. Source and
extracted files are never buffered in full. I/O buffers are bounded, while
metadata scales with file count and zstd uses its own working memory. Metadata
is limited to 64 MiB; payload offsets and sizes remain `u64` and have no such limit.
Frame corruption is reported during extraction, so a failed installation may
leave a partial file. The existing lack of rollback is unchanged.
`core_config.path_scope` accepts `"user"`, `"system"`, or `"user_choice"`.
Omission uses user scope. Fixed policies show no scope selector; `user_choice`
shows user/system radio options when the PATH task is checked, defaulting to user.
`InstallOptions.add_to_path` carries the selected `PathScope`, or `None` when
disabled. The operation rejects a scope forbidden by the package policy before
writing files. System PATH requires launching setup with Run as administrator;
there is no automatic UAC relaunch/helper yet. Access, value type, and PATH length
are checked before extracting files. A system PATH directory should not be
writable by ordinary users. Per-user tasks apply to the account running setup.
Launch and release-note completion checkboxes are shown only in previews until
runtime launch actions are implemented.
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

## Remaining installation work

Add directory-preserving packaging and parent-directory creation, automatic
elevation for protected operations, cancellation, and rollback. The
current operation overwrites existing files and can leave a partial installation
after failure. Keep GPUI and event channels in the UI adapter.
Headless GUI tests exercise real payload installation, failure/retry, progress,
and the close guard. Native Windows appearance and accessibility still need the
manual checks above.

## Windows integration

Desktop, Start Menu, and autostart use native `.lnk` files in the current user's
Desktop, Programs, and Startup known folders. They point to the installed main
executable with the installation folder as the working directory. Existing links
to the same target are preserved; unrelated links are not overwritten.

PATH updates append the installation directory to `HKCU\Environment` for user
scope, or the Session Manager environment key in `HKLM` for system scope. Existing
entries, `%VARIABLE%` references, and `REG_SZ`/`REG_EXPAND_SZ` types are preserved.
Duplicate checks account for case, separators, quotes, and expanded references.
Directories containing `;`, `%`, or NUL cannot safely be added and are rejected.
A bounded `WM_SETTINGCHANGE` broadcast notifies other applications. Existing
processes may keep their old environment; reopen terminals after installation.

`InstallOperation::run` stays async. Blocking Windows calls run via
`tokio::task::spawn_blocking`; events are emitted on the coordinating task. Task
errors propagate and prevent a successful `Finished` event. Completed files and
integration changes are not rolled back on a later failure.

Native tests use temporary shortcut files and uniquely named registry keys under
`HKCU\Software\SIMP\Tests`, never the real PATH or shortcut folders. A sandbox
that denies registry writes cannot run those three registry tests without granting
access; the remaining native tests only read known folders and process elevation.
