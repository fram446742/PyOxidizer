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
| Per-package library files        | `just pyoxy-package ...` | `dist/pyoxy-modules/*.packed-resources` | n/a (data files) |
| Development runner               | `just pyoxy-dev ...`     | runs `target/debug/pyoxy`      | dynamic (system libpython)|
| Standalone `python`              | `just python `[X.Y]      | `dist/python/python`           | static (embedded)         |

### Static, self-contained runner (`just pyoxy`)

The shipped mode: a single-file executable that statically links a Python
distribution and embeds its standard library, needs no Python installation,
and starts with an empty `sys.path` (hermetic — it imports only what it
carries).

```console
$ just pyoxy                        # 115 MB release binary into dist/pyoxy/
$ just pyoxy 3.12                   # choose the Python version
${DIST}/pyoxy -V
Python 3.10.22
```

Under the hood (what `just` runs):

```console
$ cargo run --release --quiet --bin pyoxidizer -- \
    pyoxy --path pyoxy --dest dist/pyoxy [--var PYTHON_VERSION X.Y]
```

> `pyoxidizer pyoxy` evaluates the `resources` Starlark target of
> `pyoxy/pyoxidizer.bzl` and then drives cargo with `PYO3_CONFIG_FILE` for
> you. If you edit that `.bzl` file, delete `pyoxy/build/` first — the
> Starlark target cache does not detect `.bzl` edits.

### Shipping Python libraries

Third-party packages can become importable to pyoxy in two ways, and both
can be combined.

**One file per package (`just pyoxy-package`)** — the flexible way. pip
resolves each requirement (including transitive dependencies) and every
package is written to its own removable file:

```console
$ just pyoxy-package requests lxml
wrote dist/pyoxy-modules/requests.packed-resources
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

**Baked into the executable (`just pyoxy` libraries argument)** — the
single-file way. A space separated list of requirements is baked at build
time:

```console
$ just pyoxy 3.10 "requests pyyaml"
$ cargo run ... -- pyoxy ... --var REQUIREMENTS "requests pyyaml"   # raw form
```

Result: one file containing interpreter, standard library and packages.

**Constraint of both ways:** resources are loaded from memory, so anything
that cannot load that way — C extension modules and untyped files — is
skipped with a warning at generation time. Pure Python packages
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

## Tests

```console
$ just test
```

runs the test suites of the `pyoxidizer` build tool (including pip wheels
and CLI snapshots) and of the pyoxy runner (interpreter, YAML and the
`pyoxy-modules` loader cases).

## PyOxy is part of PyOxidizer

PyOxy is part of the
[PyOxidizer](https://github.com/indygreg/PyOxidizer.git) project. However,
PyOxy is intended to be useful as a standalone project/tool and is developed as
such. However, it leverages a lot of functionality developed for the PyOxidizer
project and its canonical source repository is the aforementioned PyOxidizer
repository. The interpreter configuration and its many knobs are documented in
`docs/pyoxy_overview.rst` and `docs/pyoxy_yaml.rst`.
