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

#[cfg(stdlib_packed_resources)]
const STDLIB_RESOURCES_DATA: Option<&[u8]> =
    Some(include_bytes!(env!("PYTHON_PACKED_RESOURCES_PATH")));

#[cfg(not(stdlib_packed_resources))]
const STDLIB_RESOURCES_DATA: Option<&[u8]> = None;

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
        if let Some(resources) = STDLIB_RESOURCES_DATA {
            // Register the packed resources and enable oxidized_importer to load them.
            self.packed_resources
                .push(PackedResourcesSource::Memory(resources));
            self.oxidized_importer = true;

            // The default sys.path doesn't exist in self-contained binaries. So
            // prevent it from materializing by default. But don't overwrite a custom
            // search path setting if one is already defined!
            if self.interpreter_config.module_search_paths.is_none() {
                self.interpreter_config.module_search_paths = Some(vec![]);
            }

            // Without this, Python will attempt to derive these automatically and will likely
            // emit warnings about the paths not existing.
            if let Ok(exe) = std::env::current_exe() {
                if let Some(exe_dir) = exe.parent() {
                    self.interpreter_config.home = Some(exe_dir.to_path_buf());
                    self.interpreter_config.prefix = Some(exe_dir.to_path_buf());
                    self.interpreter_config.exec_prefix = Some(exe_dir.to_path_buf());
                }
            }
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
