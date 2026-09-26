# AGENTS.md

## Project Overview

SIMP (Simple Installers Made Painless) is a Windows installer creation tool written in Rust.

The goal is to provide a much simpler alternative to installer-authoring systems such as WiX for applications that have relatively straightforward installation requirements.

SIMP should favor:

- Simple declarative configuration
- Sensible defaults
- Minimal boilerplate
- A small and understandable package format
- Clear separation between installer logic and UI
- Rust-first implementation
- Incremental development rather than premature abstraction

The initial target is Windows.

---

## Workspace Structure

The project is a Cargo workspace.

Expected high-level structure:

    simp/
    ├── Cargo.toml
    ├── simp-core/
    ├── simp-cli/
    ├── simp-installer/
    └── simp-gui/          # may be added later

### simp-core

Shared domain types and functionality used across workspace crates.

Examples:

- Installer configuration types
- Manifest/package metadata
- Installation target types
- Package/footer definitions
- Serialization/deserialization
- Validation
- Shared error types

Do not turn `simp-core` into a generic utilities crate.

Functionality should only live here when it represents shared SIMP concepts or genuinely needs to be shared between crates.

### simp-cli

Developer-facing command-line application.

Expected eventual usage:

    simp build simp.json
    simp validate simp.json

Its primary responsibility is turning a SIMP project configuration and application files into a distributable installer executable.

The binary produced by this crate should be named `simp`.

### simp-installer

The generic installer executable/runtime.

This executable is used as the bootstrap executable when building an installer.

The final installer conceptually consists of:

    [simp-installer executable]
    [manifest / package payload]
    [footer]

The installer runtime opens its own executable, reads the fixed-size footer from the end of the file, validates it, obtains offsets into the package, and then reads the manifest/payload.

The installer contains both installation logic and its end-user GUI.

These should initially be separated through modules rather than separate crates.

Installation/domain logic must not depend on UI code.

For example:

    simp-installer/src/
    ├── main.rs
    ├── install/
    ├── ui/
    └── platform/

Do not introduce separate installer-core / installer-ui crates unless there is a concrete reason to do so.

### simp-gui

Optional/future developer-facing visual configuration editor.

This is NOT the graphical UI displayed by the generated installer.

Its purpose is to provide a visual alternative to manually editing `simp.json`.

It should operate on the same configuration/domain model used by `simp-cli`.

Do not implement this unless explicitly requested.

---

## Installer Construction

Do not rebuild or recompile the installer runtime for every application.

`simp-installer` is compiled normally into a generic Windows executable.

`simp-cli` constructs the final installer approximately as follows:

1. Obtain/copy the precompiled `simp-installer` executable.
2. Read and validate the user's SIMP configuration.
3. Gather the requested application files.
4. Construct the package/manifest.
5. Append the package data to the installer executable.
6. Append a fixed-size footer.

Conceptually:

    +----------------------------+
    | Windows PE / installer EXE |
    +----------------------------+
    | Manifest / package data    |
    | Application payload        |
    +----------------------------+
    | Fixed-size SIMP footer     |
    +----------------------------+

Windows permits additional data after the PE executable.

The footer exists so the runtime does not need to scan the executable looking for magic bytes.

Instead it can:

1. Determine its own executable path.
2. Open itself as a file.
3. Seek backwards by the known footer size.
4. Read the footer.
5. Validate its magic value and format version.
6. Read offsets/sizes from the footer.
7. Seek directly to the relevant package data.

---

## Binary Format

Treat the appended data format as a real binary file format.

Do not rely on Rust struct memory layout or directly transmute structs to/from bytes.

Encode/decode fields explicitly.

Use explicit endianness, currently little-endian.

Use fixed-width integer types in persistent/on-disk structures:

    u8
    u16
    u32
    u64

Do NOT use architecture-dependent types such as `usize` for serialized offsets or sizes.

For example, an offset into the installer should generally be represented as `u64`, even if it ultimately needs conversion to another type for an API call.

### Magic Value

Magic bytes identify SIMP data and are a property of the format, not per-manifest state.

Prefer an associated constant, for example:

    impl Manifest {
        pub const MAGIC: u32 = ...;
    }

Do not expose magic bytes as something users configure.

### Parsing

Binary parsing should be panic-free for untrusted or malformed installer/package data.

Avoid `unwrap()` and `expect()` in parsing code.

Prefer checked access and error propagation.

For example:

    value
        .get(4..6)
        .ok_or(ParseError::UnexpectedEof)?
        .try_into()
        .map_err(|_| ParseError::InvalidField)?

Prefer meaningful error types over `()` as parsing functionality grows.

---

## Configuration

The user-facing project configuration is intended to use JSON.

Example eventual concept:

    {
      "name": "Example",
      "version": "1.0.0",
      "install_target": "program_files",
      "files": [...]
    }

The exact schema is still evolving.

Do not invent large configuration schemas without being asked.

Configuration should describe user intent rather than expose Windows Installer/MSI implementation details.

SIMP should eventually support concepts such as:

- Application metadata
- Default installation location
- Files/directories to install
- Start Menu shortcuts
- Desktop shortcuts
- Registry entries
- Autostart
- Uninstallation

Not all of these need to exist in the initial implementation.

---

## Installation Targets

Represent well-known installation locations semantically rather than baking arbitrary paths into configuration.

For example:

    pub enum InstallTarget {
        ProgramFiles,
        LocalAppData,
        Custom(...),
    }

The enum represents the concept.

Platform-specific code is responsible for resolving it to an actual filesystem path.

Do not assume `%APPDATA%/Local`.

On Windows:

- `%APPDATA%` normally represents Roaming AppData.
- `%LOCALAPPDATA%` represents Local AppData.

A typical per-user application installation location is based on Local AppData.

---

## Installation Operations

Installation logic should be modeled independently of its presentation.

A likely high-level abstraction is an installation operation:

    InstallOperation

which receives the information necessary to perform an installation and exposes a synchronous operation such as:

    run(...)

The core installation logic should NOT:

- Create its own GUI
- Know about egui/GPUI
- Assume it runs on the UI thread
- Spawn threads merely because a GUI may need asynchronous behavior

The caller decides where/how the operation runs.

---

## Events and UI Separation

Installation logic should expose progress through semantic events.

For example:

    pub enum InstallEvent {
        Started,
        CreatingDirectory { ... },
        InstallingFile { ... },
        CreatingShortcut { ... },
        Progress { completed: u64, total: u64 },
        RollingBack { ... },
        Finished,
    }

The exact event model may evolve.

The installation layer should communicate through an abstraction such as an event sink/callback rather than directly manipulating UI state.

For example:

    pub trait EventSink {
        fn emit(&mut self, event: InstallEvent);
    }

A GUI can adapt this to channels or whatever mechanism its framework prefers.

A CLI can adapt the same events to terminal output.

Do not make the installation core depend directly on `mpsc`, egui, GPUI, or another frontend technology unless there is a concrete reason.

---

## Cancellation and Future User Interaction

The first implementation does not need sophisticated interactive installation behavior.

However, avoid architecture that prevents it later.

Installation operations should eventually be able to support:

- Cancellation
- Questions requiring user input
- Retry/abort decisions
- Requests to close running applications

Cancellation can initially be represented by a small abstraction that the operation periodically checks.

Do not implement an elaborate bidirectional event system prematurely.

Leave a clean extension point instead.

---

## Transactions and Rollback

Installation should eventually support rollback when an operation fails.

Think in terms of explicit installation operations rather than arbitrary side effects.

Examples:

- Create directory
- Install file
- Replace file
- Create shortcut
- Write registry value

Operations should eventually contain enough information to undo completed changes where practical.

V1 may implement only a subset of this.

Rollback should be best-effort and should report failures rather than panic.

Do not assume rollback itself cannot fail.

---

## Uninstallation

SIMP should eventually support uninstalling applications it installed.

The design should therefore distinguish between:

- Files owned by the installation
- User-created/application data that must not automatically be deleted

Do not recursively delete an installation directory merely because SIMP originally created it unless ownership of its contents is known.

An installation state/record may eventually be written during installation and consumed by the uninstaller.

The exact mechanism is not yet finalized.

---

## Dependencies

V1 does not need dependency/bootstrapper support.

For example, SIMP does not initially need to install:

- .NET runtimes
- Visual C++ redistributables
- WebView2
- Other third-party prerequisites

Applications can initially be expected to ship self-contained where necessary.

Do not implement dependency management unless explicitly requested.

---

## MSI / MSIX

The initial output format is a custom installer EXE.

Do not introduce MSI or MSIX as a requirement.

The architecture should ideally allow alternative packaging backends in the future, but do not abstract around hypothetical MSI/MSIX support prematurely.

---

## Rust Guidelines

Prefer straightforward, idiomatic Rust.

Priorities:

1. Correctness
2. Readability
3. Explicit behavior
4. Useful errors
5. Testability
6. Performance

Avoid clever abstractions without a demonstrated need.

Avoid panics for recoverable errors.

In particular, data read from:

- configuration files
- installer executables
- manifests
- appended payloads

must be treated as potentially malformed.

Prefer `Result` and meaningful error types.

Use `Path` / `PathBuf` for filesystem paths rather than treating paths as strings internally.

Use fixed-width integers for serialized data.

---

## Development Approach

Work vertically and incrementally.

The first useful milestone should be something similar to:

    simp build simp.json

producing:

    Example-Setup.exe

which can be launched and successfully install a small test application.

Prioritize this complete path over implementing every planned feature.

A reasonable initial sequence is:

1. Define minimal configuration.
2. Define minimal footer/package format.
3. Build generic installer executable.
4. Make CLI append package + footer.
5. Make installer locate/read its own footer.
6. Parse manifest.
7. Extract/copy payload.
8. Add minimal installer UI.
9. Add richer Windows integration incrementally.

Do not build the visual configuration editor before the CLI-to-installer vertical slice works unless explicitly requested.

---

## Scope Discipline

SIMP intentionally starts small.

When implementing a requested feature:

- Do not redesign unrelated parts of the project.
- Do not introduce speculative abstractions.
- Do not add dependencies merely for convenience when the standard library is sufficient.
- Do not implement future features just because this document mentions them.
- Preserve clear extension points where future requirements are already known.

If a requested change conflicts with the architecture described here, explain the conflict before performing a large refactor.
