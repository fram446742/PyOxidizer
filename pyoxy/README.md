# PyOxy

PyOxy is a Python runner. PyOxy enables you to run Python interpreters with far
more control and features than `python`.

## Shipping Python libraries

PyOxy can import third-party Python packages two ways:

* **`pyoxy-modules/` directory** — one `*.packed-resources` file per Python
  package, produced by `pyoxidizer package`, copied next to the executable.
  Copying a file in makes the library importable, deleting the file removes
  it, with no rebuild required:

  ```
  $ pyoxidizer package --dest pyoxy-modules requests
  $ ./pyoxy -c 'import requests; print(requests.__version__)'
  ```

* **Baked in at build time** — `pyoxidizer pyoxy` builds the self-contained
  runner and can embed packages into the single executable:

  ```
  $ pyoxidizer pyoxy --path pyoxy --dest dist/pyoxy --var REQUIREMENTS "requests pyyaml"
  ```

See `docs/pyoxy_overview.rst` for details, including the in-memory loading
constraint (pure Python packages are the sweet spot).

PyOxy is part of the
[PyOxidizer](https://github.com/indygreg/PyOxidizer.git) project. However,
PyOxy is intended to be useful as a standalone project/tool and is developed as
such. However, it leverages a lot of functionality developed for the PyOxidizer
project and its canonical source repository is the aforementioned PyOxidizer
repository.
