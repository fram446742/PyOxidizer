// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Tests of the `pyoxy-modules` loader: every `*.packed-resources` file
//! next to the executable is registered as a resource source, so copying a
//! file in makes a library importable and deleting it removes the library.

use {
    anyhow::{Context, Result},
    assert_cmd::{cargo::cargo_bin, Command},
    libtest_mimic::{Arguments, Trial},
    predicates::prelude::*,
    python_packed_resources::Resource,
    std::{borrow::Cow, path::Path},
};

/// Write a packed resources file containing a single module whose source is
/// compiled by the importer when the module is first imported.
fn write_module(path: &Path, module_name: &str, source: &str) -> Result<()> {
    let resource = Resource {
        name: Cow::from(module_name),
        is_python_module: true,
        in_memory_source: Some(Cow::from(source.as_bytes())),
        ..Resource::default()
    };

    let mut data = Vec::new();
    python_packed_resources::write_packed_resources_v3(&[resource], &mut data, None)?;

    std::fs::write(path, data).with_context(|| format!("writing {}", path.display()))
}

fn run() -> Result<()> {
    // The loader reads the directory next to current_exe(), so the executable
    // is copied into an isolated directory like the masquerade tests do.
    let td = tempfile::Builder::new()
        .prefix("pyoxy-modules-test-")
        .tempdir()?;
    let test_exe = td
        .path()
        .join(format!("pyoxy{}", if cfg!(windows) { ".exe" } else { "" }));
    std::fs::copy(cargo_bin("pyoxy"), &test_exe).context("copying pyoxy executable")?;

    // Files without the packed resources suffix are ignored: the loader must
    // not attempt to parse them.
    let modules_dir = td.path().join("pyoxy-modules");
    std::fs::create_dir(&modules_dir).context("creating pyoxy-modules directory")?;
    std::fs::write(modules_dir.join("README.txt"), b"not resources")?;

    write_module(
        &modules_dir.join("test_module.packed-resources"),
        "pyoxy_loader_test_module",
        "MESSAGE = 'loaded from pyoxy-modules'\n",
    )?;

    // Copying the file in makes the library importable.
    Command::new(&test_exe)
        .arg("-c")
        .arg("import pyoxy_loader_test_module; print(pyoxy_loader_test_module.MESSAGE)")
        .assert()
        .success()
        .stdout(predicate::eq("loaded from pyoxy-modules\n").normalize());

    // Deleting it removes the library again.
    std::fs::remove_file(modules_dir.join("test_module.packed-resources"))?;

    Command::new(&test_exe)
        .arg("-c")
        .arg("import pyoxy_loader_test_module")
        .assert()
        .failure()
        .stderr(predicate::str::contains("ModuleNotFoundError"));

    Ok(())
}

fn main() {
    let args = Arguments::from_args();
    let test = Trial::test("modules", move || run().map_err(Into::into));
    libtest_mimic::run(&args, vec![test]).exit();
}
