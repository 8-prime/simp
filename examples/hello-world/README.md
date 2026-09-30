# Hello World package

This example packages a small Windows command file and a text file using the
current SIMP JSON schema. `AppData` selects the per-user location beneath
`%LOCALAPPDATA%\Programs\SIMP Example`.

Build the runtime and CLI from the workspace root, in this order:

```powershell
cargo build -p simp-installer --release
cargo build -p simp-cli --release
```

Then build this package:

```powershell
Push-Location examples/hello-world
../../target/release/simp-cli.exe --input simp.json --output installer.exe
Pop-Location
```

Run the CLI from this example directory because component paths currently resolve
relative to the working directory. The CLI embeds the release installer when it
is compiled, so rebuild it after changes to the installer runtime.

`installer.exe` contains the runtime, both payload files, the manifest, and the
SIMP footer. It can open the setup wizard and read the package. The current runtime
UI installs the payload into the selected folder when Install is clicked.
Optional Windows integration tasks are omitted from this example.

The generated executable is ignored by Git and can be rebuilt with the commands
above.
