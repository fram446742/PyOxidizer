// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Snapshots the runner's own command line surface.
//!
//! Interpreter arguments belong to CPython and are covered by the `python`
//! test; these cases pin the parts pyoxy defines itself: the runner help text
//! and the errors for malformed runner options.

#[test]
fn cli_tests() {
    trycmd::TestCases::new()
        .default_bin_path(env!("CARGO_BIN_EXE_pyoxy"))
        .case("tests/cmd/*.trycmd");
}
