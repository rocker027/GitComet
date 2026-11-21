## Contributing

### Developer Certificate of Origin

RevWeave is licensed under AGPL-3.0-only. By submitting a contribution you agree that it is licensed under AGPL-3.0-only, the same license as the project (inbound = outbound). Contributors keep the copyright in their work.

Every commit must carry a `Signed-off-by:` line. Add it with `git commit -s`. The line certifies the [Developer Certificate of Origin](https://developercertificate.org/): you wrote the contribution or otherwise have the right to submit it under the project's license.

#### Third-party material and employer-owned contributions

Identify copied or adapted third-party code, documentation, images, and other material in the pull request. Include its source, copyright notices, license, and the affected files. A sign-off cannot grant rights you do not hold, and it does not automatically change a dependency's license.

Do not submit employer-owned or other third-party-owned material as if you personally own it. Obtain the necessary permission from the rights holder before you submit it.

Preserve the actual authors and co-authors of contributions. Ask maintainers for help with an unrecognized GitHub account or an incorrectly attributed commit; do not remove attribution to make a check pass.

Questions about contribution permissions: open an issue on the [RevWeave repository](https://github.com/rocker027/RevWeave/issues). Do not include private details in public issue comments.

### AI tools

You may use AI tools for your contribution. The [AI policy](AI_POLICY.md) sets what that requires, starting with your own review of every line before you submit it.

### Workspace layout

See [Architecture](docs/architecture.md) for dependency boundaries, state flow,
repository lifetimes, and document ownership.

- `crates/gitcomet-base`: product identity (`identity`), per-user directories (`platform::dirs`), platform detection, file utilities, panic containment (`panic`), operation traces and work counters. No Git domain types; core re-exports these modules at their `gitcomet_core::` paths.
- `crates/gitcomet-text`: text foundations without GPUI itself: rope and text model, tab expansion, instrumented LRU caches, syntax token kinds, date and time formatting, and cached path display. The UI kit re-exports `rope` and `text_model`.
- `crates/gitcomet-syntax`: tree-sitter syntax highlighting without GPUI: language detection, the bundled grammars and queries, prepared documents parsed off the UI thread, live documents for editors, injections, occurrences and pairs. Renderers style its token kinds through a `SyntaxPalette`.
- `crates/gitcomet-conflict-resolver`: the conflict resolver's model without GPUI: marker parsing, two- and three-way row projections, navigation, provenance, resolution and autosolve summaries, and word highlights. Rendering and styled-text caches stay in the UI host.
- `crates/gitcomet-markdown`: the markdown preview model without GPUI: parsing, HTML and table flattening, inline spans, heading anchors, code-fence languages, row wrapping and scrollbar marker buckets. Rendering and row styling stay in the UI host.
- `crates/gitcomet-core`: domain types, Git service contracts, merge algorithm, conflict session, text utils.
- `crates/gitcomet-git-gix`: `gix`/gitoxide backend implementation.
- `crates/gitcomet-state`: MVU state store, reducers, effects, conflict session management.
- `crates/gitcomet-fs-watch`: native macOS filesystem watcher support used by the state store, plus Windows test support.
- `crates/gitcomet-diagrams`: offline Mermaid recognition and rendering, independent of GPUI.
- `crates/gitcomet-fonts`: bundled font assets shared by diagram rendering and the UI kit.
- `crates/gitcomet-tree-sitter-alloc`: installs the allocator hooks used by tree-sitter.
- `crates/gitcomet-perf`: performance measurement records (sidecars) written by benchmarks and probes, and the `perf_budget_report` binary that checks them against budgets.
- `crates/gitcomet-ui-kit`: GPUI foundations reusable without the app: runtime policy (`ui_runtime`, installed live by the app at launch, deterministic otherwise), appearance, UI scale, themes, fonts, interaction primitives, text inputs, tooltips, icons, client-side window chrome (frame, title-bar geometry and drag, resize edges, window controls), and components (buttons, pickers, menus, settings rows, navigation tabs, interstitials). Builds on `gitcomet-base`, with no dependency on core, state, the application, or the UI host.
- `crates/gitcomet-ui-terminal`: the embedded terminal widget: an alacritty backend on a PTY and the GPUI viewport that paints it, selects text and forwards input. Terminal tabs, sessions, shutdown prompts and preferences stay in the UI host.
- `crates/gitcomet-ui-media`: image and diagram previews: device-pixel image inspection with tiled resampling, SVG rasterization with the bundled fonts, asynchronous diagram surfaces and the shared source/rendered toggle. The host supplies inputs, opens the menus the toolbars request and saves sampling preferences.
- `crates/gitcomet-extension-api`: the contract for compiled-in extensions: contributions (repository views, status items, settings pages, commands, key bindings, menu entries, assets, entry gates, close guards), weak window and repository handles, and per-extension session/workspace storage. Depends on core, state, and the UI kit, never on the UI host.
- `crates/gitcomet-ui-gpui`: the RevWeave window: views, panes, panels (focused diff/merge windows, conflict resolver, word diff); `UiLaunch` opens the browser window and hosts extensions (`view/extension_host.rs`). With none registered it installs nothing.
- `crates/gitcomet-app`: process launch (`AppLaunch`): identity install, crash reporting, CLI (clap), browser-instance broker, difftool/mergetool/setup/uninstall modes. GUI dependencies are optional (`ui-gpui` feature).
- `crates/gitcomet`: the executable: allocator, platform resources, packaging, and instrumentation binaries.
- `crates/gitcomet-extension-example` and `crates/gitcomet-extension-example-app`: a neutral example product built only on public upstream interfaces, with an extension registering most kinds of contribution; the UI host's tests run it (`view/tests/extensions.rs`), and CI builds the product in its own context.

Product names, identifiers, and links come from `gitcomet_core::identity`, never
from string literals: `scripts/ci/identity_literals.py` fails on new ones.

Declare dependencies for workspace members in the root `[workspace.dependencies]`
table and inherit them with `workspace = true`, including build, development and
target-specific dependencies. Keep optional flags and features needed by a
particular member in that member's manifest.

### Getting started

Linux prerequisites:

- Install the C toolchain, Clang, `pkg-config`, and the UI development libraries. On Ubuntu (tested on 26.04):

  ```bash
  sudo apt install build-essential clang pkg-config \
    libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libxkbcommon-x11-dev
  ```

  The `-dev` packages pull in their runtime libraries (`libfontconfig1`, `libxkbcommon0`, `libxkbcommon-x11-0`) and `libxcb1-dev`. On other distributions, install the equivalent packages (for example `sudo dnf install clang` on Fedora or `sudo pacman -S clang` on Arch).
- Install mold 3.0.0 and its `ld.mold` alias on your `PATH`. The repository installer downloads checksum-pinned upstream binaries for x86_64 and ARM64 Linux:

  ```bash
  python3 scripts/install-mold.py --bin-dir "$HOME/.local/bin"
  export PATH="$HOME/.local/bin:$PATH"
  mold --version
  ```

Cargo uses `scripts/linux/mold-linker.sh` to select mold through Clang for all Linux build profiles, including tests, coverage, and profiling. Linux releases use the same pinned installer. See the [mold 3.0.0 release](https://github.com/rui314/mold/releases/tag/v3.0.0) for upstream binaries and source installation on other architectures.

To verify the linker after building, run `readelf -p .comment target/debug/revweave`; it should include `mold 3.0.0`. To temporarily use the system linker on x86_64 Linux, run `CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc cargo build` (use `CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER` on ARM64).

Windows prerequisites (Windows 10/11):

- Install Visual Studio 2022 (Community or Build Tools).
- Install the `Desktop development with C++` workload.
- Ensure both MSVC tools and Windows 10/11 SDK components are installed.
- This repo configures Cargo to use `scripts/windows/windows-lld-linker.cmd` for x64 and ARM64 Windows builds. The wrapper uses the active Rust toolchain's bundled `rust-lld` linker and discovers the MSVC and Windows SDK libraries, so `cargo build` works from a regular PowerShell/CMD shell. No separate LLVM installation is needed; the Visual Studio components above are still required.

The default build includes the GPUI app and the gix backend:

```bash
cargo build --locked
```

To build the command-line app without GPUI:

```bash
cargo build --locked -p gitcomet --no-default-features --features gix
```

Run (opens the repo passed as the first arg, or falls back to the current directory):

```bash
cargo run --locked -p gitcomet --bin revweave -- /path/to/repo
```

### Testing

The [CI runner](scripts/ci/run.py) separates package feature contexts and runs
GPUI suites in their libtest process. Use the `cargo-nextest` version
pinned in [install-nextest.py](scripts/ci/install-nextest.py). RevWeave has no hosted CI, so run these checks locally before you submit.
Install Git LFS and initialize the syntax fixture submodule before running tests:

```bash
git lfs version
git submodule update --init fixtures/syntax_test
```

RevWeave has no hosted CI, so run the checks below locally before you submit.
Compile and run the workspace tests, including the UI, with:

```bash
python3 scripts/ci/run.py compile --context workspace
python3 scripts/ci/run.py test --context workspace
python3 scripts/ci/run.py doc --context workspace
```

Use `--context core`, `diagrams`, `state`, `backend`, `app`, `app-gui`, or `ui` for a focused
run. The `app` context disables GUI features; `app-gui` covers browser startup,
terminal detachment, and the browser-instance broker in the application crates.
Run the same commands with
`--context example` to check the downstream extension product in its separate
feature context. Recompile a context after changing its code.

Large unit-test suites use private child modules. Tests move with the code they
test, including into a new crate: use [inventory.py](scripts/ci/inventory.py) to
snapshot the compiled harnesses before and after the change, then compare names
and ignore flags modulo the documented module-prefix mapping. Record intentional
additions or replacements separately from mechanical moves; retain the existing
GPUI process isolation.

The backend's `test-support` feature exposes test Git environments and mergetool
consent overrides. In-tree tests enable it through dev dependencies; external test
harnesses must opt in. Normal app builds leave it disabled. See the
[profiling guide](scripts/profiling/README.md#backend-measurements) for probe builds.

Formatting, Clippy, dependency boundaries, grammar symbols and the licence list (matching the package contexts of `scripts/ci/`):

```bash
cargo fmt --all --check
cargo clippy --locked -p gitcomet-core -p gitcomet-diagrams -p gitcomet-state -p gitcomet-git-gix -p gitcomet-tree-sitter-alloc -p difftastic -- -D warnings
cargo clippy --locked -p gitcomet -p gitcomet-app --no-default-features --features gix -- -D warnings
cargo clippy --locked -p gitcomet-ui-kit -p gitcomet-extension-api -p gitcomet-extension-example -p gitcomet-ui-gpui -p gitcomet-app -- -D warnings
python3 scripts/ci/boundaries.py
python3 scripts/ci/grammar_symbols.py
bash scripts/generate-open-source-licenses.sh --check
python3 scripts/generate-bundled-license-notices.py --check
```

After a dependency change, run `scripts/generate-open-source-licenses.sh` to refresh the open-source licence list the app shows.
After changing a checked-in font or vendored license or notice, run `python3 scripts/generate-bundled-license-notices.py` to refresh the file included in packages. This file covers checked-in fonts and vendor notices only; it does not establish that every dependency or asset notice has been collected.

Coverage:

```bash
rustup component add llvm-tools-preview
cargo install --locked cargo-llvm-cov
bash scripts/coverage.sh
```

This writes:

- `target/llvm-cov/lcov.info` (machine-readable report)
- `target/llvm-cov/html/index.html` (local detailed report)

### Release packaging

macOS packaging is handled by:

```bash
scripts/package-macos.sh --version 0.2.0 --arch arm64 --release
scripts/package-macos.sh --version 0.2.0 --arch x86_64 --release
```

Use `--skip-dmg` when running in restricted/sandboxed environments where `hdiutil create` is unavailable.

Release artifacts are named `revweave-v<version>-<os>-<arch>.<ext>`. Build them locally with `scripts/package-macos.sh` and `scripts/package-linux.sh`. RevWeave has no release workflow, Homebrew tap, AUR or APT deployment.

Every GitHub release must also carry the Corresponding Source that AGPL-3.0 requires. Create the source archive from the exact commit used to build the binaries, then verify and publish it alongside them:

```bash
scripts/package-source.sh --commit "v${VERSION}^{commit}" --version "$VERSION" --out dist
scripts/generate-open-source-licenses.sh --check
python3 scripts/generate-bundled-license-notices.py --check
```

The source archive command needs Git, Cargo and network access to fetch the pinned fixture and any missing dependencies. It does not include uncommitted changes. Its extracted fixture is already present; do not run `git submodule update` inside the archive.

The app's list contains license identifiers, not full notices. The bundled notice file covers the checked-in fonts and vendored component license files; review the remaining Cargo dependencies, other assets, and their applicable notices before releasing. Verify every binary's source commit and the contents of the actual platform packages. A source archive or a passing list check alone does not establish release compliance.

### Linux packages

`scripts/package-linux.sh` builds the release artifacts from one staged payload. The `.deb`, `.rpm` and AppImage include the binary, desktop entry, all five icon sizes, licence files and README. The tarball includes the binary, README, `LICENSE-AGPL-3.0`, `NOTICE` and `BUNDLED-LICENSE-NOTICES.txt` under a `revweave-v<version>-linux-<arch>/` directory. Package metadata lives in `packaging/linux/`: `debian-control.in` for the `.deb` and `revweave.spec` for the `.rpm`.

Linked libraries are declared automatically by `dpkg-shlibdeps` and rpm AutoReq, and explicitly in the package templates. Libraries the app loads at runtime (Vulkan, EGL, Wayland) are declared by hand. `scripts/check-linux-runtime-deps.sh` fails when the binary's libraries or its glibc baseline drift from those declarations. Update both package templates when the runtime dependencies change.

Pass `--revision` (`.deb`) and `--release` (`.rpm`) to `scripts/package-linux.sh` to set the package revision (the `1` in `1.2.3-1`). Raise them when rebuilding packages for an existing version.

The RPM supports Fedora 42 and newer.
