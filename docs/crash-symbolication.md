# Crash Symbolication

RevWeave ships stripped binaries, so a crash report is a list of hex addresses
until it is resolved against debug symbols. Symbols live in a **Breakpad symbol
store**: static files keyed by each build's debug ID, so a dump can be
symbolized long after the build that produced it. RevWeave does not publish a
public symbol store. Whoever builds a release generates the symbols and keeps
them (see "Generating symbols locally" below).

- [`scripts/install-dump-syms.sh`](../scripts/install-dump-syms.sh) installs a pinned
  [`mozilla/dump_syms`](https://github.com/mozilla/dump_syms) release.
- [`scripts/emit-breakpad-symbols.sh`](../scripts/emit-breakpad-symbols.sh) converts a
  binary, `.pdb` or `.dSYM` into a `.sym` file, writes it into the store layout, and
  fails the build if the result is empty.
- [`scripts/symbolicate-minidump.sh`](../scripts/symbolicate-minidump.sh) resolves a
  minidump against a symbol store.

## The Two Kinds of Crash Report

**RevWeave's own crash log.** Written by the panic hook in
[`crates/gitcomet-app/src/crashlog.rs`](../crates/gitcomet-app/src/crashlog.rs) whenever a
Rust panic occurs, except one contained by `gitcomet_base::panic::contain`, which
the caller turns into a fallback:

| OS | Location |
| --- | --- |
| Windows | `%LOCALAPPDATA%\revweave\crashes\panic-<pid>-<ms>.log` |
| macOS | `~/Library/Logs/revweave/crashes/panic-<pid>-<ms>.log` |
| Linux | `${XDG_STATE_HOME:-~/.local/state}/revweave/crashes/panic-<pid>-<ms>.log` |

The `location=<file>#L<line>` line in that file is **always usable without any
symbols** — it is compile-time information baked into the binary. Ask for this
file first; it often identifies the panic outright.

The backtrace below it depends on the platform. Linux and macOS ship with
`strip = "debuginfo"`, keeping the symbol table, so frames carry function names
(no line numbers). **Windows frames stay `<unknown>`**: MSVC keeps symbols in the
`.pdb`, which is not shipped, and `Backtrace::force_capture` resolves through
dbghelp against that file. On Windows, rely on the `location=` line and on
symbolicating the WER minidump.

**Operating-system dumps.** Windows Error Reporting minidumps, and any minidump a
future in-process crash handler writes. These need the symbol store.

Note that some crashes never reach the panic hook — a double panic aborts before
the hook runs, and a `__fastfail` bypasses every user-mode handler. If a process
died but wrote no crash log, that absence is itself a clue.

## Symbolicating a Minidump

```sh
cargo install minidump-stackwalk --locked
scripts/symbolicate-minidump.sh path/to/crash.dmp
```

No public store is published, so point the script at your own store: set
`GITCOMET_SYMBOLS_URL` (or pass `--symbols-url`) to the base URL of a store you
populated with `emit-breakpad-symbols.sh`, for example a local mirror served over
HTTP. The script has no default URL.

Symbols are fetched lazily over HTTP and cached under
`${XDG_CACHE_HOME:-~/.cache}/revweave-symbols`, so only the modules a crash
actually touches are downloaded, and only once per build.


Reading the output: a `Crash reason` of `EXCEPTION_STACK_BUFFER_OVERRUN` with
`FAST_FAIL_FATAL_APP_EXIT` and an `int 0x29` instruction is **not** a buffer
overrun — it is a deliberate Rust `abort`, reached by `std::process::abort`, a
double panic, a panic crossing an `extern "C"` boundary, or an allocation
failure. Frames marked *"Found by: stack scanning"* are guesses.

## What the Release Build Produces

A release build can emit `<name>/<debug-id>/<name>.sym` with
`emit-breakpad-symbols.sh`. Keep those files together with the release artifacts
of the same version.

The debug ID survives stripping on every platform — ELF `.note.gnu.build-id`,
the PE debug directory's PDB GUID+Age, and Mach-O `LC_UUID` — which is why
shipped binaries stay stripped and remain symbolizable.

Symbols are built with `--inlines`. RevWeave uses fat LTO, so without inline
records most frames collapse into whichever outer function survived inlining.
The cost is real: on x86_64 Linux the `.sym` is ~239 MiB (~43 MiB gzipped)
against ~37 MiB without inline records. Storage is a few cents a month; drop the
flag in `emit-breakpad-symbols.sh` if that ever changes.

### Generating symbols locally

`[profile.release]` carries no debug info, so `cargo bench`, which inherits that
profile, is not taxed with fat-LTO debug info it discards. Generating a build's
symbols means opting in on the command line, and the input differs per platform,
because each one keeps its debug info somewhere else.

**Linux** — DWARF is in the binary, so it must not be stripped before extraction:

```sh
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only CARGO_PROFILE_RELEASE_STRIP=none \
  cargo build -p gitcomet --release --locked --features ui-gpui,gix --bin revweave
scripts/emit-breakpad-symbols.sh --input target/release/revweave --store /tmp/symstore
```

**macOS** — `dsymutil` must run, and the input is the bundle, not the binary:

```sh
CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed \
  cargo build -p gitcomet --release --locked --features ui-gpui,gix --bin revweave
scripts/emit-breakpad-symbols.sh --input target/release/revweave.dSYM \
  --store /tmp/symstore --arch "$(uname -m)" --module-name revweave \
  --allow-missing-cfi
```

`--module-name` is not optional here. `dump_syms` names the module after the
file it is handed, and the store path follows that name, so the bundle produces
`revweave.dSYM/<DEBUG_ID>/revweave.dSYM.sym` and the hashed Mach-O inside it
produces `revweave-<hash>/...`. A stackwalker asks for
`<module>/<DEBUG_ID>/<module>.sym` using the name the crash report carries —
the loaded executable, `Contents/MacOS/revweave` — so either spelling is a 404
and the symbols may as well not have been published. Linux and Windows need no
override: their inputs are already named `revweave` and `revweave.pdb`.

**Windows** — the input is the `.pdb`; `strip` is irrelevant on MSVC:

```pwsh
$env:CARGO_PROFILE_RELEASE_DEBUG = "line-tables-only"
cargo build -p gitcomet --release --locked --features ui-gpui,gix --bin revweave
bash scripts/emit-breakpad-symbols.sh --input target/release/revweave.pdb --store /tmp/symstore
```

Pointing the script at a stripped binary, or at a macOS binary instead of its
`.dSYM`, is rejected by the guards rather than yielding symbols that resolve
nothing — the message names stripping as the likely cause, so check the input
first.

### Unwind data on macOS

`emit-breakpad-symbols.sh` requires STACK (CFI) records, because without them a
stackwalker falls back to scanning — the "Found by: stack scanning" frames that
make a report unreadable. macOS is the exception and runs with
`--allow-missing-cfi`: `dsymutil` puts DWARF in the `.dSYM` but leaves
`__eh_frame` in the linked binary, and `dump_syms` reads a single file with no
macOS equivalent of the `.pdb`/`.exe` re-pairing it performs on Windows.

The macOS run therefore gets no STACK records, so macOS stacks may unwind by
scanning. Recovering CFI would mean also dumping the linked binary and merging
its STACK records into the same debug ID.

For debuggers that cannot read Breakpad `.sym`, keep the Windows `.pdb` and the
macOS `.dSYM` of each release yourself. Archive the `.dSYM` as a tar file so the
bundle wrapper survives.

Note that `target/release/revweave.dSYM` is a *symlink* into the build
intermediates, not the bundle itself, because Cargo uplifts it that way.
`package-macos.sh` replaces it with a real directory before the build
intermediates are cleaned, because `tar` stores a dangling symlink without
complaint and would archive an empty bundle.


Note what these do **not** contain. Everything here is built at
`debug = "line-tables-only"`, which carries function names, file, line and
inline frames but no variables, parameters or types — measured on a probe build,
zero `DW_TAG_variable` / `DW_TAG_formal_parameter` / `DW_TAG_structure_type`
against 37 / 62 / 34 at `debug = 2`. So WinDbg and lldb will resolve a stack but
cannot inspect state. Raising the level would not help the store either: a
Breakpad `.sym` generated from a full-debug build is the same size with the same
record counts, because the format has nowhere to put types. And release builds
are `opt-level = 3` with fat LTO, where even full debug info yields
`<optimized out>` for most parameters and "No locals." The Linux raw DWARF is not
archived — it is ~203 MiB per build and the `.sym` covers `minidump-stackwalk`.

## Limits

- **Symbols cannot be produced retroactively.** Any release built without keeping its
  symbols has none and never will.
- **Linux and macOS have no minidump writer.** The store pays off immediately on
  Windows, where WER writes dumps. On the other platforms it improves crash
  *logs* via the retained symbol table; producing real minidumps there would
  need an in-process crash handler.
