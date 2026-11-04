// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use {
    anyhow::Result,
    pyembed::{MainPythonInterpreter, OxidizedPythonInterpreterConfig, PackedResourcesSource},
    serde::{Deserialize, Serialize},
    std::{
        ffi::{OsStr, OsString},
        ops::{Deref, DerefMut},
        path::Path,
    },
};

/// The packed resources file(s) compiled into this executable.
#[cfg(stdlib_packed_resources)]
const STDLIB_RESOURCES_DATA: Option<&[u8]> =
    Some(include_bytes!(env!("PYTHON_PACKED_RESOURCES_PATH")));

#[cfg(not(stdlib_packed_resources))]
const STDLIB_RESOURCES_DATA: Option<&[u8]> = None;

/// Directory next to the executable scanned for per-library resources.
///
/// Each `*.packed-resources` file in it is registered as a resource source:
/// copying a file in makes the library importable, deleting the file removes
/// it. This is the mechanism behind `pyoxidizer package`.
const MODULES_DIR: &str = "pyoxy-modules";

const MODULES_FILE_SUFFIX: &str = ".packed-resources";

fn add_module_files<'a>(builder: &mut pyembed::OxidizedPythonInterpreterConfig<'a>, dir: &Path) {
    let mut paths: Vec<_> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .map(|n| n.to_string_lossy().ends_with(MODULES_FILE_SUFFIX))
                        .unwrap_or(false)
            })
            .collect(),
        // The directory does not have to exist.
        Err(_) => return,
    };
    paths.sort();

    if paths.is_empty() {
        return;
    }

    for path in paths {
        // Memory-mapped so a large library set costs no Rust memory and files
        // can be added and removed while files are being copied around.
        builder
            .packed_resources
            .push(PackedResourcesSource::MemoryMappedPath(path));
    }

    builder.oxidized_importer = true;
}

/// An embedded Python interpreter configuration.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default, transparent)]
pub struct Config<'a> {
    inner: OxidizedPythonInterpreterConfig<'a>,
}

impl<'a> Deref for Config<'a> {
    type Target = OxidizedPythonInterpreterConfig<'a>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<'a> DerefMut for Config<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'a> From<Config<'a>> for OxidizedPythonInterpreterConfig<'a> {
    fn from(c: Config<'a>) -> Self {
        c.inner
    }
}

impl<'a> Config<'a> {
    /// Apply the current environment's settings to this config.
    pub fn apply_environment(&mut self) {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()));

        if let Some(resources) = STDLIB_RESOURCES_DATA {
            // Register the packed resources and enable oxidized_importer to load them.
            self.inner
                .packed_resources
                .push(PackedResourcesSource::Memory(resources));
            self.inner.oxidized_importer = true;

            // The default sys.path doesn't exist in self-contained binaries. So
            // prevent it from materializing by default. But don't overwrite a custom
            // search path setting if one is already defined!
            if self.interpreter_config.module_search_paths.is_none() {
                self.interpreter_config.module_search_paths = Some(vec![]);
            }

            // Without this, Python will attempt to derive these automatically and will likely
            // emit warnings about the paths not existing.
            if let Some(exe_dir) = &exe_dir {
                self.interpreter_config.home = Some(exe_dir.to_path_buf());
                self.interpreter_config.prefix = Some(exe_dir.to_path_buf());
                self.interpreter_config.exec_prefix = Some(exe_dir.to_path_buf());
            }
        }

        // Per-library resources shipped as files next to the executable. Also
        // applies to a dynamic libpython build: that is how libraries are
        // usually tried out during development.
        if let Some(exe_dir) = &exe_dir {
            add_module_files(&mut self.inner, &exe_dir.join(MODULES_DIR));
        }

        self.set_missing_path_configuration = false;
    }
}

/// Construct and run an embedded Python interpreter.
///
/// CPython declines to initialize for some command line arguments - notably
/// `--help` and `--version` - and requests a clean process exit instead. That
/// arrives as an error carrying an exit code rather than a message, so surface
/// it as a normal return value and let the caller exit with that code.
pub fn run_interpreter(config: OxidizedPythonInterpreterConfig) -> Result<i32> {
    let interp = match MainPythonInterpreter::new(config) {
        Ok(interp) => interp,
        Err(err) => {
            let Some(code) = err.exit_code() else {
                return Err(anyhow::Error::new(err).context("initializing Python interpreter"));
            };

            // The interpreter never finished starting, so nothing flushed the
            // streams for us. `--help` writes to stdout, which would otherwise
            // be lost.
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let _ = std::io::Write::flush(&mut std::io::stderr());

            return Ok(code);
        }
    };

    Ok(interp.run())
}

/// Runs an embedded Python interpreter in `python` mode.
pub fn run_python<T>(exe: &Path, args: &[T]) -> Result<i32>
where
    T: Into<OsString> + AsRef<OsStr>,
{
    let mut config = Config::default();
    config.apply_environment();
    config.exe = Some(exe.to_path_buf());
    config.argv = Some(
        vec![exe.as_os_str().to_os_string()]
            .into_iter()
            .chain(args.iter().map(|x| x.into()))
            .collect::<Vec<_>>(),
    );

    let config: OxidizedPythonInterpreterConfig = config.into();

    run_interpreter(config)
}
