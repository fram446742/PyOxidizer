.. _pyoxy_overview:

========
Overview
========

The ``pyoxy`` Executable
========================

PyOxy is distributed as a ``pyoxy`` compiled executable. This executable
links against a Python implementation/distribution (i.e. ``libpython``).

The Python implementation/distribution and any resources defined in its standard
library *may* be compiled statically into the ``pyoxy`` executable. This
enables ``pyoxy`` to function as a single file Python distribution. (This is
how official builds of ``pyoxy`` are distributed.)

``pyoxy``'s ``int main()`` is implemented in Rust. By default it behaves like a
``python`` executable: the process arguments are handed to an embedded Python
interpreter, with no subcommand and no special file name required. Runner
features are reachable through long options ``python`` does not define, so
they can never be mistaken for a script name; run ``pyoxy --runner-help``
for their documentation.

Full Python Interpreter Control
===============================

``pyoxy --yaml`` (see :ref:`pyoxy_yaml`) gives you very low-level
control over the behavior of the Python interpreter: much lower than what is
possible with ``python`` command arguments or environment variables.

This control can be useful for iterating/testing on different Python embedding
configurations (such as how you would need to configure PyOxidizer). The control
can also be useful for use in automated testing where you may want to simulate
an embedded Python configuration but don't want to produce your own executable
for each configuration variation. With ``pyoxy --yaml``, you can simply define
a YAML file defining the interpreter configuration and use a single executable
for driving the Python interpreter N ways.

Shipping Python Libraries
=========================

A ``pyoxy`` executable can import third-party Python packages in two ways.

``pyoxy-modules`` Directory
---------------------------

Every ``*.packed-resources`` file in a ``pyoxy-modules`` directory next to the
executable is registered as a resource source and its contents are importable,
like built-in modules. Files are memory-mapped, so a large library set costs no
readiness memory and files can be swapped while copying.

The files are produced one per Python package by the ``pyoxidizer package``
command (or, in this repository, ``just pyoxy-package``)::

   $ pyoxidizer package --dest pyoxy-modules --python-version 3.11 requests
   $ cp pyoxy-modules/requests.packed-resources pyoxy-modules/

Packages resolved this way include every transitive dependency, each in its own
file: copying a file in makes the library importable, deleting the file removes
it, and no rebuild is required either way. Files without the
``.packed-resources`` suffix in that directory are ignored.

Since the resources are loaded from memory, resources that cannot be loaded
that way - C extension modules and untyped files - are skipped. Each affected
package reports an incomplete file and the command ends with a
``packaging complete`` / ``packaging incomplete`` verdict naming it. Modules
loaded this way also have no ``__file__``, so packages that read it at import
time fail with ``NameError``; pure Python packages that avoid both are the
sweet spot for this mechanism.

Each file is accompanied by a ``<name>.packed-resources.json`` sidecar recording
the Python version and target triple it was built for. A ``pyoxy`` built for a
different Python skips the mismatch with a warning on stderr rather than failing
on incompatible bytecode; files without a sidecar are loaded as before.

Baking Libraries Into the Executable
------------------------------------

Libraries can instead be baked into the single executable at build time by the
``pyoxidizer pyoxy`` (or ``just pyoxy <python-version> "<libraries>"``)
command::

   $ pyoxidizer pyoxy --path pyoxy --dest dist/pyoxy --var REQUIREMENTS "requests pyyaml"

This produces a single file containing the interpreter, the standard library
and the named packages. The same in-memory constraint applies: C extension
modules are skipped with a warning.

Passing ``--split-resources`` to ``pyoxidizer pyoxy`` writes the resources to a
``pyoxy-resources.packed-resources`` file next to the executable instead, which
the runner memory-maps: the executable shrinks from ~150 MB to ~23 MB at the
cost of shipping both files together. Neither mode needs a Python installation
on the host.

The two mechanisms compose: a baked binary still picks up ``pyoxy-modules/``
files (``just pyoxy`` copies ``dist/pyoxy-modules`` next to the binary on
every build), which is a convenient way to iterate on additional libraries
without rebuilding the (large) executable.

Additional Python Features
==========================

``pyoxy`` supplements the built-in features of ``python`` with its own.

With ``pyoxy``, you can:

* Dynamically choose from the system, jemalloc, mimalloc, or snmalloc memory
  allocators.
* Easily leverage the ``oxidized_importer`` extension module for importing
  Python modules and loading file-based resources faster than the official
  importers in the Python standard library.
* Automatically discover the location of the ``terminfo`` database at runtime,
  helping to ensure terminal functionality works as intended.
* Automatically write a file containing a list of imported modules when the
  Python interpreter finalizes.
* And more.

``pyoxy`` aims to expose all the value-added features implemented in the
``pyembed`` Rust crate via the CLI so Python developers can harness these
features without having to use something more heavyweight, like PyOxidizer.

Behaving Like ``python``
========================

``pyoxy`` behaves like a ``python`` executable by default:
``pyoxy -c "print('hello, world')"`` works as you would expect, as do
``-m``, script files, ``-`` for stdin, ``-O``, ``-X``, ``--version`` and the
rest of the interpreter's arguments, with no subcommand and regardless of the
executable's file name. This enables ``pyoxy`` to be used as a drop-in
replacement for ``python``.

Because ``-h``, ``--help`` and ``-V`` belong to the interpreter, the runner's
own features are documented by ``pyoxy --runner-help`` and are exposed as long
options ``python`` does not define (currently ``--yaml``). The deprecated
subcommand spellings ``run-python`` and ``run-yaml`` are still accepted.

.. code-block::

   $ ./pyoxy -c "print('hello, world')"
   hello, world
   $ ./pyoxy
   Python 3.10.9 (...) on linux
   Type "help", "copyright", "credits" or "license" for more information.
   >>>
