# Scripts

All three source `env.ps1` themselves, and can be run from anywhere.

- **`build.ps1`** - `cargo build --release`.
- **`clean.ps1`** - `-Target` after changing `Cargo.toml`'s
  `[package.metadata.esp-idf-sys]` section, `sdkconfig.defaults` or
  anything under `components/`; a plain `cargo clean` only clears the
  host-side cache and leaves the CMake config stale, so those changes
  appear to have no effect. `-Full` wipes the build directory entirely and
  forces an ESP-IDF re-clone.
- **`flash.ps1`** - flashes and monitors in one step, auto-detecting the
  COM port. `-CaptureSeconds N` captures to a log instead of running
  interactively.
