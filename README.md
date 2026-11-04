# PyOxidizer

[![Build Status](https://github.com/indygreg/PyOxidizer/workflows/.github/workflows/pyoxidizer.yml/badge.svg)](https://github.com/indygreg/PyOxidizer/actions)

`PyOxidizer` is a utility for producing binaries that embed Python.
The over-arching goal of `PyOxidizer` is to make complex packaging and
distribution problems simple so application maintainers can focus on
building applications instead of toiling with build systems and packaging
tools.

`PyOxidizer` is capable of producing a single file executable - with
a copy of Python and all its dependencies statically linked and all
resources (like `.pyc` files) embedded in the executable. You can
copy a single executable file to another machine and run a Python
application contained within. It *just works*.

`PyOxidizer` exposes its lower level functionality for embedding
self-contained Python interpreters as a tool and software library. So if
you don't want to ship executables that only consist of a Python
application, you can still use `PyOxidizer` to e.g. produce a library
containing Python suitable for linking in any application or use
`PyOxidizer`'s embedding library directly for embedding Python in a
larger application.

The _Oxidizer_ part of the name comes from Rust: executables produced
by `PyOxidizer` are compiled from Rust and Rust code is responsible
for managing the embedded Python interpreter and all its operations.
If you don't know Rust, that's OK: PyOxidizer tries to make the existence
of Rust nearly invisible to end-users.

While solving packaging and distribution problems is the primary goal
of `PyOxidizer`, a side-effect of solving that problem with Rust is
that `PyOxidizer` can serve as a bridge between these two languages.
`PyOxidizer` can be used to easily add a Python interpreter to _any_
Rust project. But the opposite is also true: `PyOxidizer` can also be
used to add Rust to Python. Using `PyOxidizer`, you could _bootstrap_
a new Rust project which contains an embedded version of Python and your
application. Initially, your project is a few lines of Rust that
instantiates a Python interpreter and runs Python code. Over time,
functionality could be (re)written in Rust and your previously
Python-only project could leverage Rust and its diverse ecosystem. Since
`PyOxidizer` abstracts the Python interpreter away, this could all be
invisible to end-users: you could rewrite an application from Python to
Rust and people may not even know because they never see a `libpython`,
`.py` files, etc.

## Project Info

:house: The official home of the `PyOxidizer` project is
https://github.com/indygreg/PyOxidizer.

:notebook_with_decorative_cover: Documentation (generated from the `docs/` directory) is available
at https://gregoryszorc.com/docs/pyoxidizer/main/.

:speech_balloon: The [pyoxidizer-users](https://groups.google.com/forum/#!forum/pyoxidizer-users)
mailing list is a forum for users to discuss all things PyOxidizer.

:moneybag: If you want to financially contribute to PyOxidizer, do so
via [GitHub Sponsors](https://github.com/sponsors/indygreg) or
[on Patreon](https://www.patreon.com/indygreg).


## Shipping Python with this fork

This fork adds a self-contained Python runner (`pyoxy`) and one uniform build
interface on top of PyOxidizer's packaging library. Everything user-facing goes
through `just`; the equivalent `pyoxidizer` invocations are listed in
`pyoxy/README.md` for CI and for machines without `just`.

The rule is: **the interpreter and its standard library stay inside the
binary, statically linked; third-party libraries stay outside, modular.**

| What you want | Command | Result |
|---|---|---|
| Self-contained runner (default) | `just pyoxy 3.11 "requests"` | one ~154 MB file: libpython static, standard library and packages embedded |
| Same, payload beside the binary | `just pyoxy 3.11 "requests" x86_64-unknown-linux-gnu 3 split` | ~23 MB file + `pyoxy-resources.packed-resources` (~131 MB) |
| Modular library, packed | `just pyoxy-package 3.11 six` | `dist/pyoxy-modules/six.packed-resources` + `.json` |
| Modular library, unpacked | `just pyoxy-package-dir 3.11 numpy` | `dist/pyoxy-modules/numpy/` (real files) + `.json` |
| Iterating on the runner | `just pyoxy-dev -c 'print(1)'` | dynamic build against the host's libpython |
| Standalone interpreter | `just python 3.11` | `dist/python/python` |

`just pyoxy` copies `dist/pyoxy-modules/` next to the built binary on every
build, so the runner and its libraries are always in the same directory.
Neither mode needs a Python installation on the target machine.

`just clean` removes what these commands generate (`dist/`, `pyoxy/build/`,
`pyoxy/target/`) and keeps cargo's shared `target/` cache; `just clean-all`
drops that cache too. Both are reproducible by re-running the recipes.

### Which library mode can I use?

| Package kind | Packed (`pyoxy-package`) | Unpacked (`pyoxy-package-dir`) |
|---|---|---|
| Pure Python (`six`, `colorama`, `requests`, `pyyaml`) | yes | yes |
| Needs a real `__file__` | no | yes |
| C extension modules (`numpy`, `pandas`, `psutil`, `capstone`, `pyocd`) | no | yes |

Packed resources are loaded from memory, and loading a shared library from
memory is only implemented on Windows: on Linux the importer refuses it. That
is not a size or grouping problem, so splitting a packed file does not help -
those packages must be unpacked. A package whose optional accelerator is
dropped (`pyyaml`, `wcwidth`) still imports from its packed form, without the
accelerated path.

Unpacked directories are plain `sys.path` entries: `<dest>/<pkg>/` contains the
wheel's real files (modules, `.so`, data, `.dist-info`), so extension modules
and `__file__` behave exactly as in a normal virtualenv. Sizes are the unpacked
wheels: `numpy` 59 MB, `pandas` 44 MB, `pyocd` 20 MB.

### How each mode is wired

* **Embedded (default).** The `resources` target of `pyoxy/pyoxidizer.bzl`
  writes a packed resources blob into the build directory, and
  `pyoxy/build.rs` compiles it into the binary with `include_bytes!`
  (`cfg(stdlib_packed_resources)`). The runner registers it as an in-memory
  resource source and starts with an empty `sys.path`.
* **Split.** `pyoxidizer pyoxy --split-resources` sets
  `PYOXIDIZER_PACKED_RESOURCES_SPLIT=1` for the build, which makes the build
  script emit `cfg(stdlib_packed_resources_split)` instead of embedding. The
  runner then memory-maps `pyoxy-resources.packed-resources` from its own
  directory.
* **Packed libraries.** Every `pyoxy-modules/*.packed-resources` is
  memory-mapped and registered with the oxidized importer. Files are
  version-checked against the interpreter: the `.json` sidecar written by
  `pyoxidizer package` records the Python version, and a mismatch is skipped
  with a warning instead of failing on incompatible bytecode.
* **Unpacked libraries.** Every subdirectory of `pyoxy-modules/` is added to
  `module_search_paths` (self-contained builds) or to `PYTHONPATH` (dynamic
  builds), so CPython's own `PathFinder` imports from disk. A directory wins
  over a `*.packed-resources` file of the same name, with a warning.
* **Development runner.** `pyoxy-dev` links the host's libpython dynamically:
  it starts instantly, but inherits the host's `sys.path` and is only for
  iterating on the runner, never for distribution.

### Reading the output

Every command ends with a verdict; nothing important is left implicit.

| Output | Meaning | What to do |
|---|---|---|
| `ok: dist/pyoxy/pyoxy - embedded (single file), baked libraries: …; N modular librar(y/ies) beside it` | the runner was built | nothing |
| `packaging complete: N package(s), nothing was skipped` | every packed file is whole | nothing |
| `packaging incomplete: N of M package(s) are missing resources` followed by the two commands to run | some packages lost C extension modules; a packed file cannot carry them here | run the printed `--unpack` command |
| `incomplete: N C extension module(s) skipped` (per package) | that file is partial | use the unpacked mode for it |
| `pyoxy: skipping <file>: built for Python 3.10, this runner is Python 3.11` | the library and the runner target different interpreters | rebuild with the runner's version |
| `pyoxy: using <pkg>/ and ignoring <pkg>.packed-resources` | both forms are present; the directory wins | delete the stale packed file |
| `warning: not shipping extension module _tkinter …` | that module needs Tcl/Tk shared libraries this static distribution cannot link | expected; `tkinter` is unavailable in built binaries |
| `warning: license text for … is missing from the Python distribution archive` | the upstream archive lacks that file | licensing output only; the build is unaffected |
| `warning: N embedded modules read __file__ …` | in-memory resources have no path | ship those packages unpacked when they need `__file__` |
| `pyoxy: <blob> not found next to the executable` | a split build lost its payload file | keep `pyoxy` and `pyoxy-resources.packed-resources` together |

### Constraints worth knowing

* Wheel ABI is pinned: `cp311` wheels need a 3.11 runner, and `manylinux`
  wheels need a compatible glibc. The `.json` sidecar records both so a mixed
  set fails loudly.
* Editing `pyoxy/pyoxidizer.bzl` does not invalidate the Starlark target
  cache: delete `pyoxy/build/` after changing it.
* Release builds use thin LTO, one codegen unit and symbol stripping
  (`[profile.release]` in `Cargo.toml`); the optimization level can be passed
  per build as the fourth argument of `just pyoxy`.
