.. _pyoxy_history:

===============
Project History
===============

Blog Posts
==========

* `Announcing the PyOxy Python Runner <https://gregoryszorc.com/blog/2022/05/10/announcing-the-pyoxy-python-runner/>`_ (2022-05-10)

Changelog
=========

0.3.0
-----

Not yet released.

* ``pyoxy`` now behaves like a ``python`` executable by default: the process
  arguments are handed to the embedded Python interpreter, with no subcommand
  and no special file name required. Runner features are exposed as long
  options ``python`` does not define: ``--yaml FILE`` runs a YAML-configured
  interpreter (replacing the ``run-yaml`` subcommand) and ``--runner-help``
  documents them, since ``-h``/``--help`` now belong to the interpreter.
  The ``run-python`` and ``run-yaml`` subcommands are deprecated spellings of
  the default behavior and ``--yaml`` respectively and are still accepted.
* Official release artifacts now contain Python 3.11, 3.12, 3.13, 3.14 and
  3.15 variants. The Python 3.9 and 3.10 variants are no longer provided, as
  PyOxidizer dropped support for these versions.

0.2.0
-----

Released on 2022-05-30.

* Official release artifacts now contain Python 3.8 and 3.10 variants.
  Previously, only Python 3.9 was provided.
* The Sphinx documentation now contains extensive documentation for the
  Python interpreter configuration structs and enums. The content is derived
  from the canonical Rust source code. This should make it easier to
  understand the fields in YAML configurations without having to consult
  Rust crate docs.
* Release artifacts are now ``.tar.gz`` files and contain a ``COPYING``
  file with licensing annotations.
* The release mechanism for PyOxy is now streamlined, hopefully enabling
  faster releases going forward.

0.1.0
-----

Released 2022-05-10.