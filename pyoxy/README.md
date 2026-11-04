# PyOxy

PyOxy is a Python runner. PyOxy enables you to run Python interpreters with far
more control and features than `python`.

By default `pyoxy` behaves exactly like a `python` executable: no subcommand
and no special file name required. Runner features are reachable through long
options `python` does not define — run `pyoxy --runner-help` to print them:

```console
$ ./pyoxy -c 'print("hello, world")'
$ ./pyoxy --yaml myapp.yaml          # run a YAML-configured interpreter
$ ./pyoxy --runner-help              # document the runner's own features
```

All building and day-to-day use is done with `just` recipes defined in the
repository's `Justfile`. Each recipe is a thin wrapper over
`cargo run --bin pyoxidizer -- <command>`; the raw `pyoxidizer` invocations
are listed per section for environments without `just` (e.g. CI uses them).

## Build modes

| Mode                             | Command                  | Output                         | Python linkage            |
|----------------------------------|--------------------------|--------------------------------|---------------------------|
| Self-contained runner            | `just pyoxy `[`libs`]    | `dist/pyoxy/pyoxy`             | static (embedded)         |
| Split-resources runner           | `just pyoxy … split`     | `dist/pyoxy/pyoxy` + `pyoxy-resources.packed-resources` | static (blob beside the exe) |
| Per-package library files        | `just pyoxy-package ...` | `dist/pyoxy-modules/*.packed-resources` | n/a (data files) |
| Development runner               | `just pyoxy-dev ...`     | runs `target/debug/pyoxy`      | dynamic (system libpython)|
| Standalone `python`              | `just python `[X.Y]      | `dist/python/python`           | static (embedded)         |

Rust optimization settings for these builds are described in
[Optimizations](#optimizations).

### Static, self-contained runner (`just pyoxy`)

The shipped mode: a single-file executable that statically links a Python
distribution and embeds its standard library, needs no Python installation,
and starts with an empty `sys.path` (hermetic — it imports only what it
carries).

```console
$ just pyoxy                        # release binary into dist/pyoxy/ (3.11)
$ just pyoxy 3.12                   # choose the Python version
$ just pyoxy 3.11 "requests"        # additionally bake libraries into it
$ just pyoxy 3.11 "requests" x86_64-unknown-linux-gnu z   # + size-leaning Rust profile
$ just pyoxy 3.11 "" "" "" split    # small binary, resources in a sibling file
${DIST}/pyoxy -V
Python 3.11.17
```

Size is dominated by what you embed, not by the Rust code: CPython 3.11.17
with `requests` baked is 153 MB, of which the packed resources blob is
131 MB and the linked `libpython3.a` another 43 MB on disk. See
[Optimizations](#optimizations).

Under the hood (what `just` runs):

```console
$ cargo run --release --quiet --bin pyoxidizer -- \
    pyoxy --path pyoxy --dest dist/pyoxy [--var PYTHON_VERSION X.Y]
```

> `pyoxidizer pyoxy` evaluates the `resources` Starlark target of
> `pyoxy/pyoxidizer.bzl` and then drives cargo with `PYO3_CONFIG_FILE` for
> you. If you edit that `.bzl` file, delete `pyoxy/build/` first — the
> Starlark target cache does not detect `.bzl` edits.

#### Split resources (small binary)

Embedding the standard library is what makes the executable portable, and
also what makes it ~150 MB. `--split-resources` moves the same blob to a
file next to the executable, which the runner memory-maps:

```console
$ just pyoxy 3.11 "requests" x86_64-unknown-linux-gnu 3 split
wrote dist/pyoxy/pyoxy
wrote dist/pyoxy/pyoxy-resources.packed-resources (sidecar resources)
$ ls -lh dist/pyoxy/
pyoxy                             23M   # libpython is still linked in; the Rust code is ~3 MB
pyoxy-resources.packed-resources 131M
$ cd /tmp && /path/to/dist/pyoxy/pyoxy -c 'import requests'   # works from anywhere
```

Raw form:

```console
$ cargo run ... -- pyoxy --split-resources --path pyoxy --dest dist/pyoxy \
    [--var PYTHON_VERSION X.Y] [--var REQUIREMENTS "requests"]
```

The two files are a unit: moving the executable without the blob leaves it
without a standard library, and it says so on stderr before CPython fails to
initialize. Both modes still need no Python installation on the host — the
difference is one file versus two. Everything else composes:
`pyoxy-modules/`, `--yaml` and the per-package files work identically.

### Shipping Python libraries

Third-party packages can become importable to pyoxy in two ways, and both
can be combined.

**One file per package (`just pyoxy-package`)** — the flexible way. pip
resolves each requirement (including transitive dependencies) and every
package is written to its own removable file. The first argument is the
Python version, so files match the runner they are meant for:

```console
$ just pyoxy-package 3.11 requests lxml
wrote dist/pyoxy-modules/requests.packed-resources
wrote dist/pyoxy-modules/requests.packed-resources.json
...

$ just pyoxy                        # builds and copies pyoxy-modules/ to
                                    # dist/pyoxy/pyoxy-modules/ automatically
$ ./dist/pyoxy/pyoxy -c 'import requests; print(requests.__version__)'
```

Files are one `*.packed-resources` per pip package: **copy a file in** to
make the library importable, **delete the file** to remove it — no rebuild
is needed either way. `just pyoxy` re-syncs the directory next to the binary
on every build, and the same files can also be listed in a `--yaml`
interpreter config via `packed_resources`.

Each file has a `<name>.packed-resources.json` sidecar with the Python
version and target triple it was built for. A runner built against another
Python skips the mismatch with a warning on stderr — bytecode from a
different interpreter would otherwise fail with an opaque import error.
Files without the sidecar (older output) are loaded as before. The check
needs to know which Python the runner targets, so it applies to builds linked
against a distribution; `just pyoxy-dev` links the host's libpython and loads
files unconditionally.

**Baked into the executable (`just pyoxy` libraries argument)** — the
single-file way. A space separated list of requirements is baked at build
time:

```console
$ just pyoxy 3.11 "requests pyyaml"
$ cargo run ... -- pyoxy ... --var REQUIREMENTS "requests pyyaml"   # raw form
```

Result: one file containing interpreter, standard library and packages.

**Constraint of both ways:** resources are loaded from memory, so anything
that cannot load that way — C extension modules and untyped files — is
skipped with a warning at generation time, and the number skipped is
summarized at the end of the command (a package whose C accelerator was
dropped still imports, so its absence is otherwise easy to miss). Pure Python packages
(`requests`, `click`, `attrs`, `pyyaml`, ...) are the sweet spot; a package
whose optional C accelerator is skipped still imports and works, just
without the accelerated path (e.g. `charset_normalizer` behind `requests`).

Note baking also recompiles the large binary, while sidecar files don't:
iterate on libraries with `pyoxy-package`, and bake final sets for
distribution.

### Development runner (`just pyoxy-dev`)

Builds no distribution: it compiles pyoxy in debug mode against the
system's `libpython` and runs it immediately. Interpreter arguments are
passed through verbatim:

```console
$ just pyoxy-dev -c 'import sys; print(sys.version)'
$ just pyoxy-dev script.py
$ just pyoxy-dev -m mymodule
```

Caveats: it links the system Python dynamically and inherits its search
paths, so imports may resolve from host site-packages — behavior you see is
not what a self-contained pyoxy would do. Use it to iterate on the runner
and interpreter configuration; validate library shipping against
`dist/pyoxy/pyoxy`.

```console
$ cargo build -p pyoxy              # raw form (build only)
```

### Standalone `python` (`just python`)

If you just want a `python` binary without the runner features:

```console
$ just python                       # -> dist/python/python
$ just python 3.14
$ cargo run ... -- python --dest dist/python [--python-version X.Y]
```

## Optimizations

Two independent axes: the Rust code, and the embedded Python payload. Only
the first is controlled by cargo; the second is what actually determines the
binary size.

### Rust code (cargo profiles)

Profiles live in the **workspace root** `Cargo.toml`. Cargo ignores
`[profile.*]` in a member manifest, which is why `pyoxy/Cargo.toml` has none
and why `pyoxidizer pyoxy` (which runs `cargo build --release --target-dir
pyoxy/target`) picks these up. Release builds use:

| Setting         | Value          | Rationale                                      |
|-----------------|----------------|------------------------------------------------|
| `opt-level`     | `3`            | runtime performance                            |
| `lto`           | `"thin"`       | cross-crate inlining for a few seconds of build |
| `codegen-units` | `1`            | best codegen, at the cost of build parallelism  |
| `strip`         | `"symbols"`    | −0.8 MB; release builds carry no debug info     |

Override the optimization level per build with the fourth `just pyoxy`
argument, which exports `CARGO_PROFILE_RELEASE_OPT_LEVEL` (the variable
`pyoxidizer pyoxy` passes through to cargo):

```console
$ just pyoxy 3.11 "" x86_64-unknown-linux-gnu z    # 3 (default), 2, 1, s, z
$ CARGO_PROFILE_RELEASE_OPT_LEVEL=z cargo build --release -p pyoxy   # raw form
```

Measured on a Rust-only dynamic build (`cargo build -p pyoxy --release`,
78 crates, no embedded payload), so the effect is isolated:

| Profile                                        | Size      | Δ         |
|------------------------------------------------|-----------|-----------|
| cargo default (`lto = false`, `codegen-units = 16`) | 3.04 MiB | —        |
| committed (`lto = "thin"`, `codegen-units = 1`, `strip`) | 2.51 MiB | −17% |
| `lto = "fat"` instead of thin                  | 2.12 MiB  | −30%      |
| `opt-level = "z"` + fat LTO + `panic = "abort"`| 1.66 MiB  | −46%      |

Startup is unchanged within noise (100 × `pyoxy -c 'pass'`: 571 ms / 570 ms
/ 573 ms for the first, second and last rows; CPython initialization
dominates at ~5.7 ms per invocation).

On the shipped binary all of this is small: CPython 3.11.17 with `requests`
baked went from 155.7 MB to 153.0 MB (−1.7%), essentially all of it `strip`,
because the Rust part is ~2% of the file. `opt-level = "z"` buys another
~0.5 MB; `panic = "abort"` is **not** enabled because it needs validation
against code relying on unwinding.

`just pyoxy-dev` and `just test` build with the dev profile and are not
affected by `[profile.release]`; the pre-existing
`[profile.dev.package."*"] opt-level = 3` in `.cargo/config.toml` only makes
dependencies faster during tests.

### Python payload (what actually grows)

- `pyoxy/pyoxidizer.bzl` enables `policy.bytecode_optimize_level_zero`,
  `..._one` and `..._two`, i.e. bytecode for `-O` and `-OO` as well. That
  triples stdlib bytecode inside the packed resources blob. Dropping the
  `-O`/`-OO` variants shrinks it a lot but breaks running the binary with
  those flags (startup imports fail on the missing optimization level).
- Libraries baked via `--var REQUIREMENTS` land in the same blob; the
  sidecar `pyoxy-modules/*.packed-resources` mode keeps them out of the
  binary entirely and is the size-friendly choice.
- libpython comes from python-build-standalone's `pgo+lto-full` flavor, so
  CPython itself is already PGO+LTO optimized upstream. No cargo setting
  applies to it.

## Tests

```console
$ just test
```

runs the test suites of the `pyoxidizer` build tool (including pip wheels
and CLI snapshots) and of the pyoxy runner (interpreter, YAML, the
`pyoxy-modules` loader cases, and `trycmd` snapshots of the runner's own
argument surface: `--runner-help` and the malformed-`--yaml` errors).

## PyOxy is part of PyOxidizer

PyOxy is part of the
[PyOxidizer](https://github.com/indygreg/PyOxidizer.git) project. However,
PyOxy is intended to be useful as a standalone project/tool and is developed as
such. However, it leverages a lot of functionality developed for the PyOxidizer
project and its canonical source repository is the aforementioned PyOxidizer
repository. The interpreter configuration and its many knobs are documented in
`docs/pyoxy_overview.rst` and `docs/pyoxy_yaml.rst`.
