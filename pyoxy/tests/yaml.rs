// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use {
    anyhow::Result,
    assert_cmd::Command,
    libtest_mimic::{Arguments, Trial},
    predicates::prelude::*,
};

fn run() -> Result<()> {
    for yaml_path in glob::glob("tests/yaml/*.yaml")? {
        let yaml_path = yaml_path?;
        let stdout_path = yaml_path.with_extension("stdout");

        assert!(stdout_path.exists());
        let expected_stdout = std::fs::read_to_string(&stdout_path)?;

        // Both spellings of the YAML runner are covered: the current
        // `--yaml` option and the deprecated `run-yaml` subcommand.
        for invocation in [["--yaml"], ["run-yaml"]] {
            Command::cargo_bin("pyoxy")?
                .args(invocation)
                .arg(&yaml_path)
                .assert()
                .success()
                .stdout(predicate::str::contains(&expected_stdout).normalize());
        }
    }

    Ok(())
}

fn main() {
    let args = Arguments::from_args();
    let test = Trial::test("main", move || run().map_err(Into::into));
    libtest_mimic::run(&args, vec![test]).exit();
}
