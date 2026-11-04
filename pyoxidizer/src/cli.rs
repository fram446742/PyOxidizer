// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use {
    crate::{
        environment::{default_target_triple, PYOXIDIZER_VERSION},
        project_building, projectmgmt, shell::with_shell,
    },
    anyhow::{anyhow, Context, Result},
    cargolike::Verbosity,
    clap::{value_parser, Arg, ArgAction, ArgMatches, Command},
    clap_complete::Shell,
    std::{collections::HashMap, path::PathBuf},
};

const BUILD_ABOUT: &str = "\
Build a PyOxidizer project.

The project directory is defined by --path and defaults to the current
directory.

Build targets to resolve can be given as positional arguments or with
--target. If none are defined, the default targets of the configuration
file are built.

This command will invoke Rust's build system tool (Cargo) to build
the project.
";

const RUN_ABOUT: &str = "\
Build and run a target in a PyOxidizer configuration file.

The project directory is defined by --path and defaults to the current
directory.

Arguments at the end of the command line are passed as extra argv to the
running application.
";

const PYTHON_ABOUT: &str = "\
Build a standalone, self-contained Python interpreter executable.

This produces a single file executable embedding a Python interpreter and
the Python standard library. It accepts the same command line arguments as
a CPython `python` executable and has no runtime dependency on a Python
installation.

Use this when you simply want an interpreter binary. To control packaging
behavior, embedded third-party packages, or interpreter startup in detail,
write a configuration file with `pyoxidizer init config` and build it with
`pyoxidizer build` instead.
";

const PYOXY_ABOUT: &str = "\
Build a self-contained pyoxy Python runner executable.

A pyoxy executable statically links a Python distribution, embeds its
standard library, and behaves like a `python` executable; see the pyoxy
documentation for its runner features.

The project directory (--path, default: the current directory) must contain
a pyoxidizer.bzl defining a `resources` target that emits Python embedding
artifacts, and a Cargo crate defining a `pyoxy` binary that consumes them:
this command runs the equivalent of:

    pyoxidizer build --path DIR --target TRIPLE resources
    cargo build --bin pyoxy --target TRIPLE   # with PYO3_CONFIG_FILE set
    cp BUILD_PATH/<triple>/<profile>/pyoxy --dest

Starlark variables are honored (--var): `PYTHON_VERSION` selects the
distribution and `REQUIREMENTS` bakes third-party packages into the single
executable, e.g.

    pyoxidizer pyoxy --path pyoxy --var REQUIREMENTS \"requests lxml\"

The resources are embedded in the executable by default, so it is one file
that works wherever it is copied. With --split-resources they are written to
a sibling `pyoxy-resources.packed-resources` file instead, which the runner
memory-maps: the executable drops from ~150 MB to ~23 MB (libpython is still
linked statically), at the cost of shipping and keeping the two files
together.

To obtain per-package files for the `pyoxy-modules` loader instead - one
file per package, addable and removable without rebuilding anything - use
`pyoxidizer package`.
";

const PACKAGE_ABOUT: &str = "\
Write one packed resources file per Python package.

For the given requirements, pip resolves each package and its dependencies
and every resolved wheel is classified and serialized to its own
`<name>.packed-resources` file in --dest. Output files are platform
independent as long as wheels are pure Python; C extension modules and
files that cannot be exported are skipped with a warning, and a summary of
what was skipped is printed at the end.

Each file is accompanied by a `<name>.packed-resources.json` recording the
Python version and target triple it was built for. A pyoxy executable skips
files it cannot load and says why, instead of failing on bytecode compiled
by a different interpreter.

The files can be loaded from memory:

* by a pyoxy executable: copy the file into the `pyoxy-modules` directory
  next to the executable (creating it if needed) and the library becomes
  importable; deleting the file removes it again.
* by any pyembed embedding: list the file in the `packed_resources` field
  of an interpreter configuration.
";

const INIT_RUST_PROJECT_ABOUT: &str = "\
Create a new Rust project embedding Python.

The PATH argument is a filesystem path that should be created to hold the
new Rust project.

This command will call `cargo init PATH` and then install files and make
modifications required to embed a Python interpreter in that application.

The new project's binary will be configured to launch a Python REPL by
default.

Created projects inherit settings such as Python distribution URLs and
dependency crate versions and locations from the PyOxidizer executable
they were created with.

On success, instructions on potential next steps are printed.
";

const INIT_CONFIG_ABOUT: &str = "\
Create a new PyOxidizer configuration file.

The PATH argument is the directory in which to write `pyoxidizer.bzl`.

The generated configuration file defines an `exe` target that builds an
executable embedding a Python interpreter and the standard library. Build
it with `pyoxidizer build --path PATH`.
";

const GENERATE_PYTHON_EMBEDDING_ARTIFACTS_ABOUT: &str = "\
Generate files useful for embedding Python in a [Rust] binary.

This low-level command can be used to write files that facilitate the
embedding of Python in a larger binary. It can be used to write:

* A custom libpython that can be linked into a binary.
* A configuration file for the PyO3 Rust crate telling it how to
  link against the aforementioned custom libpython.
* A Python packed resources file containing the entirety of the Python
  standard library.
* A Rust file defining a default `pyembed::OxidizedPythonInterpreterConfig`
  struct for configuring the embedded Python interpreter.
* tcl/tk support files (for tkinter module support).
* Microsoft Visual C++ Redistributable Runtime DLLs (Windows only).

This command essentially does what the `internal build-script` command does except
it doesn't require the presence of a PyOxidizer configuration file. Instead,
it uses an opinionated default configuration suitable for producing a set of
files suitable for common Python embedding scenarios. If the defaults are not
appropriate for your use case, you can always define a configuration file to
customize them and use `internal build-script` to produce similar output files.
";

const RUN_BUILD_SCRIPT_ABOUT: &str = "\
Runs a crate build script to generate Python artifacts.

When the Rust crate embedding Python is built, it needs to consume various
artifacts derived from processing the active PyOxidizer config file.
These files are typically generated when the crate's build script runs.

This command executes the functionality to derive various artifacts and
emits special lines that tell the Rust build system how to consume them.

Generated Rust projects invoke this automatically from their build script.
You should not normally need to run it by hand.
";

const RESOURCES_SCAN_ABOUT: &str = "\
Scan a directory or file for Python resources.

This command invokes the logic used by various PyOxidizer functionality
walking a directory tree or parsing a file and categorizing seen files.

The directory walking functionality is used by
`oxidized_importer.find_resources_in_path()` and Starlark methods like
`PythonExecutable.pip_install()` and
`PythonExecutable.read_package_root()`.

The file parsing logic is used for parsing the contents of wheels.

This command can be used to debug failures with PyOxidizer's code
for converting files/directories into strongly typed objects. This
conversion is critical for properly packaging Python applications and
bugs can result in incorrect install layouts, missing resources, etc.
";

const ANALYZE_ABOUT: &str = "\
Analyze a built binary.

Parses an executable produced by PyOxidizer and reports the embedded
Python resources, licensing information, and other metadata recorded
when it was built.
";

const LIST_TARGETS_ABOUT: &str = "\
List the build targets defined by a project's configuration file.

The project directory is defined by --path and defaults to the current
directory.
";

const CACHE_CLEAR_ABOUT: &str = "\
Clear PyOxidizer's user-specific cache.

This removes cached Python distributions and the self-managed Rust
toolchain. They will be downloaded again on next use.
";

const DIST_ABOUT: &str = "\
Inspect and extract Python distributions.

The archives these commands operate on are the standalone Python
distributions that PyOxidizer downloads and embeds into executables.
";

const DIST_INFO_ABOUT: &str = "\
Show information about a Python distribution archive.

The PATH argument is a filesystem path to an archive from a standalone
Python distribution.
";

const DIST_EXTRACT_ABOUT: &str = "\
Extract a Python distribution archive to a directory.

Either --archive-path or --download-default must be defined.
--download-default downloads the default distribution for the platform
PyOxidizer is running on.
";

const DIST_LICENSES_ABOUT: &str = "\
Show the licenses for the components of a Python distribution.

The PATH argument is a filesystem path to an archive from a standalone
Python distribution.
";

const PROJECT_LICENSING_ABOUT: &str = "\
Show licensing information for a Rust project.

The PATH argument is the directory of the Rust project to evaluate. It
reports the license of every crate the project depends on, which is what
you need before redistributing a binary built with PyOxidizer.
";

const COMPLETIONS_ABOUT: &str = "\
Print a shell completion script.

Source the output or install it into your shell's completion directory to
get command and argument completion:

    $ pyoxidizer completions bash > /etc/bash_completion.d/pyoxidizer
    $ pyoxidizer completions zsh > ~/.zfunc/_pyoxidizer
    $ pyoxidizer completions fish > ~/.config/fish/completions/pyoxidizer.fish
";

const VAR_HELP: &str = "\
Defines a single string key to set in the VARS global dict.

This argument can be used to inject variable content into the Starlark
execution context to influence evaluation.

<name> defines the key in the dict to set and <value> is its string
value.

For example, `--var my_var my_value` is functionally similar to the
Starlark expression `VARS[\"my_var\"] = \"my_value\"`.

If a Starlark variable is defined multiple times, an error occurs.
";

const ENV_VAR_HELP: &str = "\
Defines a single string key to set in the VARS global dict from an environment variable.

This is like --var except the value of the dict key comes from an
environment variable.

The <env> environment variable is read and becomes the value of the
<name> key in the VARS dict.

If the <env> environment variable is not set, the Starlark value will
be `None` instead of a `string`.

If a Starlark variable is defined multiple times, an error occurs.
";

const AFTER_HELP: &str = "\
Common tasks:

  Build a standalone Python interpreter in one command
      $ pyoxidizer python --dest ./out
      $ ./out/python -c 'import sys; print(sys.version)'

  Build the self-contained pyoxy runner
      $ pyoxidizer pyoxy --path pyoxy --dest .
      $ ./pyoxy -c 'import sys; print(sys.version)'

  Ship third-party libraries next to a runner, one file per package
      $ pyoxidizer package --dest pyoxy-modules requests
      $ ./pyoxy -c 'import requests; print(requests.__version__)'

  Create a project you can configure, then build it
      $ pyoxidizer init config myapp
      $ pyoxidizer build --path myapp

  Build and run whatever the project's default targets define
      $ pyoxidizer run --path myapp

Argument conventions:

  --path <DIR>   The project directory a command operates on. Defaults to
                 the current directory. Used by `build`, `run`, `python`
                 and `inspect targets`.

  PATH           A positional argument is the operand of a command - a
                 distribution archive, a binary to analyze, a directory to
                 create or write to. It is required wherever it exists.

  `internal` commands are invoked by generated build scripts and are not
  normally called by hand.
";

fn add_env_args(app: Command) -> Command {
    app.arg(
        Arg::new("vars")
            .long("var")
            .action(ArgAction::Append)
            .value_names(["name", "value"])
            .number_of_values(2)
            .help("Define a variable in Starlark environment")
            .long_help(VAR_HELP),
    )
    .arg(
        Arg::new("vars_env")
            .long("var-env")
            .action(ArgAction::Append)
            .value_names(["name", "env"])
            .number_of_values(2)
            .help("Define an environment variable in Starlark environment")
            .long_help(ENV_VAR_HELP),
    )
}

/// Define the `--target-triple` argument.
///
/// `with_default` controls whether the host's default triple is materialized
/// as a value. Commands forwarding this to [projectmgmt::resolve_target()]
/// leave it unset so that function retains ownership of default resolution.
/// Commands that need a `&str` ask for the default.
fn arg_target_triple(with_default: bool) -> Arg {
    let arg = Arg::new("target_triple")
        .long("target-triple")
        .action(ArgAction::Set)
        .value_name("TRIPLE")
        .help("Rust target triple to use");

    if with_default {
        arg.default_value(default_target_triple())
    } else {
        arg
    }
}

/// Define the `--path` argument for commands operating on a project directory.
fn arg_project_path(help: &'static str) -> Arg {
    Arg::new("path")
        .long("path")
        .action(ArgAction::Set)
        .value_parser(value_parser!(PathBuf))
        .default_value(".")
        .value_name("DIR")
        .help(help)
}

/// Define the positional operand `PATH` argument.
fn arg_operand_path(name: &'static str, help: &'static str) -> Arg {
    Arg::new(name)
        .action(ArgAction::Set)
        .value_parser(value_parser!(PathBuf))
        .value_name("PATH")
        .required(true)
        .help(help)
}

fn add_python_distribution_args(app: Command) -> Command {
    app.arg(arg_target_triple(true))
        .arg(
            Arg::new("flavor")
                .long("flavor")
                .action(ArgAction::Set)
                .value_name("FLAVOR")
                .default_value("standalone")
                .help("Python distribution flavor"),
        )
        .arg(
            Arg::new("python_version")
                .long("python-version")
                .action(ArgAction::Set)
                .value_name("X.Y")
                .help("Python version (X.Y) to use"),
        )
}

fn starlark_vars(args: &ArgMatches) -> Result<HashMap<String, Option<String>>> {
    let mut res = HashMap::new();

    if let Some(mut vars) = args.get_many::<String>("vars") {
        while let (Some(name), Some(value)) = (vars.next(), vars.next()) {
            if res.contains_key(name) {
                return Err(anyhow!("Starlark variable {} already defined", name));
            }

            res.insert(name.to_string(), Some(value.to_string()));
        }
    }

    if let Some(mut vars) = args.get_many::<String>("vars_env") {
        while let (Some(name), Some(env)) = (vars.next(), vars.next()) {
            if res.contains_key(name) {
                return Err(anyhow!("Starlark variable {} already defined", name));
            }

            res.insert(name.to_string(), std::env::var(env).ok());
        }
    }

    Ok(res)
}

// The command definitions below take the name to register under because every
// command is registered twice: once inside its namespace - `dist info` - and
// once as a hidden top-level command spelling the pre-namespace flat name -
// `python-distribution-info`. Both dispatch to the same code, so existing
// scripts, CI jobs and documentation keep working.

fn cmd_analyze(name: &'static str) -> Command {
    Command::new(name)
        .about("Analyze a built binary")
        .long_about(ANALYZE_ABOUT)
        .arg(arg_operand_path("path", "Path to executable to analyze"))
}

fn cmd_build() -> Command {
    add_env_args(
        Command::new("build")
            .about("Build a PyOxidizer project")
            .long_about(BUILD_ABOUT)
            .arg(arg_target_triple(false))
            .arg(
                Arg::new("release")
                    .long("release")
                    .action(ArgAction::SetTrue)
                    .help("Build a release binary"),
            )
            .arg(arg_project_path("Directory containing project to build"))
            // Positional targets predate --target and remain accepted, e.g.
            // `pyoxidizer build exe install`.
            .arg(
                Arg::new("positional_targets")
                    .action(ArgAction::Append)
                    .num_args(0..)
                    .value_name("TARGET")
                    .hide(true)
                    .help("Build target to resolve"),
            )
            .arg(
                Arg::new("targets")
                    .long("target")
                    .action(ArgAction::Append)
                    .value_name("TARGET")
                    .help("Build target to resolve"),
            ),
    )
}

fn cmd_run() -> Command {
    add_env_args(
        Command::new("run")
            .about("Build and run a target in a configuration file")
            .long_about(RUN_ABOUT)
            .trailing_var_arg(true)
            .arg(arg_target_triple(false))
            .arg(
                Arg::new("release")
                    .long("release")
                    .action(ArgAction::SetTrue)
                    .help("Build a release binary"),
            )
            .arg(arg_project_path("Directory containing project to build"))
            .arg(
                Arg::new("target")
                    .long("target")
                    .action(ArgAction::Set)
                    .value_name("TARGET")
                    .help("Build target to run"),
            )
            .arg(
                Arg::new("extra")
                    .action(ArgAction::Append)
                    .num_args(0..)
                    .value_name("ARG")
                    .help("Extra arguments to pass to the running application"),
            ),
    )
}

fn cmd_python() -> Command {
    add_python_distribution_args(
        Command::new("python")
            .about("Build a standalone, self-contained Python interpreter")
            .long_about(PYTHON_ABOUT)
            .arg(
                Arg::new("dest_path")
                    .long("dest")
                    .action(ArgAction::Set)
                    .value_parser(value_parser!(PathBuf))
                    .value_name("DIR")
                    .default_value(".")
                    .help("Directory to write the built executable to"),
            )
            .arg(
                Arg::new("name")
                    .long("name")
                    .action(ArgAction::Set)
                    .value_name("NAME")
                    .default_value("python")
                    .help("File name to write the executable as"),
            )
            .arg(
                Arg::new("release")
                    .long("release")
                    .action(ArgAction::SetTrue)
                    .help("Build a release binary"),
            ),
    )
}

fn cmd_pyoxy() -> Command {
    add_env_args(
        Command::new("pyoxy")
            .about("Build a self-contained pyoxy Python runner executable")
            .long_about(PYOXY_ABOUT)
            .arg(arg_target_triple(false))
            .arg(
                Arg::new("release")
                    .long("release")
                    .action(ArgAction::SetTrue)
                    .help("Build a release binary"),
            )
            .arg(
                Arg::new("split_resources")
                    .long("split-resources")
                    .action(ArgAction::SetTrue)
                    .help(
                        "Write the packed resources next to the executable instead of embedding them",
                    ),
            )
            .arg(arg_project_path(
                "Directory containing the pyoxy project to build",
            ))
            .arg(
                Arg::new("dest_path")
                    .long("dest")
                    .action(ArgAction::Set)
                    .value_parser(value_parser!(PathBuf))
                    .value_name("DIR")
                    .default_value(".")
                    .help("Directory to write the built executable to"),
            )
            .arg(
                Arg::new("name")
                    .long("name")
                    .action(ArgAction::Set)
                    .value_name("NAME")
                    .default_value("pyoxy")
                    .help("File name to write the executable as"),
            ),
    )
}

fn cmd_package() -> Command {
    add_python_distribution_args(
        Command::new("package")
            .about("Write one packed resources file per Python package")
            .long_about(PACKAGE_ABOUT)
            .arg(
                Arg::new("dest_path")
                    .long("dest")
                    .action(ArgAction::Set)
                    .value_parser(value_parser!(PathBuf))
                    .value_name("DIR")
                    .default_value(".")
                    .help("Directory to write the packed resources files to"),
            )
            .arg(
                Arg::new("requirements")
                    .action(ArgAction::Append)
                    .num_args(1..)
                    .value_name("REQ")
                    .required(true)
                    .help("Requirements pip can resolve, e.g. `requests` or `lxml>=5`"),
            ),
    )
}

fn cmd_cache_clear(name: &'static str) -> Command {
    Command::new(name)
        .about("Clear PyOxidizer's user-specific cache")
        .long_about(CACHE_CLEAR_ABOUT)
}

fn cmd_find_resources(name: &'static str) -> Command {
    Command::new(name)
        .about("Find resources in a file or directory")
        .long_about(RESOURCES_SCAN_ABOUT)
        .arg(
            Arg::new("distributions_dir")
                .long("distributions-dir")
                .action(ArgAction::Set)
                .value_parser(value_parser!(PathBuf))
                .value_name("DIR")
                .help("Directory to extract downloaded Python distributions into"),
        )
        .arg(
            Arg::new("scan_distribution")
                .long("scan-distribution")
                .action(ArgAction::SetTrue)
                .conflicts_with("path")
                .help("Scan the Python distribution instead of a path"),
        )
        .arg(arg_target_triple(true))
        .arg(
            Arg::new("no_classify_files")
                .long("no-classify-files")
                .action(ArgAction::SetTrue)
                .help("Whether to skip classifying files as typed resources"),
        )
        .arg(
            Arg::new("no_emit_files")
                .long("no-emit-files")
                .action(ArgAction::SetTrue)
                .help("Whether to skip emitting File resources"),
        )
        .arg(
            Arg::new("path")
                .action(ArgAction::Set)
                .value_parser(value_parser!(PathBuf))
                .value_name("PATH")
                // Omitting it is only valid with --scan-distribution.
                .required_unless_present("scan_distribution")
                .help("Filesystem path to scan for resources. Must be a directory or Python wheel"),
        )
}

fn cmd_generate_embedding_artifacts(name: &'static str) -> Command {
    add_python_distribution_args(
        Command::new(name)
            .about("Generate files for embedding Python in a Rust binary")
            .long_about(GENERATE_PYTHON_EMBEDDING_ARTIFACTS_ABOUT)
            .arg(arg_operand_path(
                "dest_path",
                "Output directory for written files",
            )),
    )
}

fn cmd_init_config_file(name: &'static str) -> Command {
    Command::new(name)
        .about("Create a new PyOxidizer configuration file")
        .long_about(INIT_CONFIG_ABOUT)
        .arg(
            Arg::new("python-code")
                .long("python-code")
                .action(ArgAction::Set)
                .value_name("CODE")
                .help("Default Python code to execute in built executable"),
        )
        .arg(
            Arg::new("pip-install")
                .long("pip-install")
                .action(ArgAction::Append)
                .value_name("PACKAGE")
                .help("Python package to install via `pip install`"),
        )
        .arg(arg_operand_path(
            "path",
            "Directory where configuration file should be created",
        ))
}

fn cmd_init_rust_project(name: &'static str) -> Command {
    Command::new(name)
        .about("Create a new Rust project embedding a Python interpreter")
        .long_about(INIT_RUST_PROJECT_ABOUT)
        .arg(arg_operand_path(
            "path",
            "Path of project directory to create",
        ))
}

fn cmd_list_targets(name: &'static str) -> Command {
    Command::new(name)
        .about("List targets available to resolve in a configuration file")
        .long_about(LIST_TARGETS_ABOUT)
        .arg(arg_project_path("Path to project to evaluate"))
}

fn cmd_distribution_extract(name: &'static str) -> Command {
    Command::new(name)
        .about("Extract a Python distribution archive to a directory")
        .long_about(DIST_EXTRACT_ABOUT)
        .arg(
            Arg::new("download_default")
                .long("download-default")
                .action(ArgAction::SetTrue)
                .conflicts_with("archive_path")
                .help("Download and extract the default distribution for this platform"),
        )
        .arg(
            Arg::new("archive_path")
                .long("archive-path")
                .action(ArgAction::Set)
                .value_parser(value_parser!(PathBuf))
                .value_name("ARCHIVE")
                .required_unless_present("download_default")
                .help("Path to a Python distribution archive"),
        )
        .arg(arg_operand_path(
            "dest_path",
            "Path to directory where distribution should be extracted",
        ))
}

fn cmd_distribution_info(name: &'static str) -> Command {
    Command::new(name)
        .about("Show information about a Python distribution archive")
        .long_about(DIST_INFO_ABOUT)
        .arg(arg_operand_path(
            "path",
            "Path to Python distribution archive to analyze",
        ))
}

fn cmd_distribution_licenses(name: &'static str) -> Command {
    Command::new(name)
        .about("Show licenses for a given Python distribution")
        .long_about(DIST_LICENSES_ABOUT)
        .arg(arg_operand_path(
            "path",
            "Path to Python distribution to analyze",
        ))
}

fn cmd_run_build_script(name: &'static str) -> Command {
    add_env_args(
        Command::new(name)
            .about("Run functionality that a build script would perform")
            .long_about(RUN_BUILD_SCRIPT_ABOUT)
            .arg(
                Arg::new("build_script_name")
                    .action(ArgAction::Set)
                    .value_name("PATH")
                    .required(true)
                    .help("Value to use for Rust build script"),
            )
            .arg(
                Arg::new("target")
                    .long("target")
                    .action(ArgAction::Set)
                    .value_name("TARGET")
                    .help("The config file target to resolve"),
            ),
    )
}

fn cmd_project_licensing(name: &'static str) -> Command {
    Command::new(name)
        .about("Show licensing information for a Rust project")
        .long_about(PROJECT_LICENSING_ABOUT)
        .arg(
            Arg::new("all_features")
                .long("all-features")
                .action(ArgAction::SetTrue)
                .help("Activate all crate features during evaluation"),
        )
        .arg(arg_target_triple(false))
        .arg(
            Arg::new("unified_license")
                .long("unified-license")
                .action(ArgAction::SetTrue)
                .help("Print a unified license document"),
        )
        .arg(arg_operand_path(
            "path",
            "The path to the Rust project to evaluate",
        ))
}

fn cmd_completions() -> Command {
    Command::new("completions")
        .about("Print a shell completion script")
        .long_about(COMPLETIONS_ABOUT)
        .arg(
            Arg::new("shell")
                .action(ArgAction::Set)
                .value_name("SHELL")
                .required(true)
                .help("Shell to generate completions for: bash, elvish, fish, powershell, zsh"),
        )
}

fn group_dist() -> Command {
    Command::new("dist")
        .about("Inspect and extract Python distributions")
        .long_about(DIST_ABOUT)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(cmd_distribution_info("info"))
        .subcommand(cmd_distribution_extract("extract"))
        .subcommand(cmd_distribution_licenses("licenses"))
}

fn group_init() -> Command {
    Command::new("init")
        .about("Create a new project")
        .long_about(
            "\
Scaffold a new project.

`init config` writes a single Starlark configuration file and is the normal
way to start a Python application. `init rust-project` creates a Rust project
embedding a Python interpreter, for when you intend to write Rust alongside
Python.
",
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(cmd_init_config_file("config"))
        .subcommand(cmd_init_rust_project("rust-project"))
}

fn group_inspect() -> Command {
    Command::new("inspect")
        .about("Analyze binaries, resources, targets and licensing")
        .long_about(
            "\
Read-only commands that explain what a project or a built artifact contains.
",
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(cmd_analyze("binary"))
        .subcommand(cmd_find_resources("resources"))
        .subcommand(cmd_list_targets("targets"))
        .subcommand(cmd_project_licensing("licensing"))
}

/// Plumbing invoked by generated Rust build scripts.
///
/// Registered but hidden: these are for debugging a build, not for daily use,
/// and they were previously advertised alongside `build`.
fn group_internal() -> Command {
    Command::new("internal")
        .about("Plumbing invoked by generated build scripts")
        .long_about(
            "\
These commands are called by the build script that `pyoxidizer init
rust-project` generates. They are documented for debugging purposes and are
not intended to be invoked directly.
",
        )
        .hide(true)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(cmd_run_build_script("build-script"))
        .subcommand(cmd_generate_embedding_artifacts("embedding-artifacts"))
}

/// Register the pre-namespace flat names as hidden top-level commands.
fn add_flat_aliases(app: Command) -> Command {
    let aliases: Vec<Command> = vec![
        cmd_analyze("analyze"),
        cmd_find_resources("find-resources"),
        cmd_list_targets("list-targets"),
        cmd_project_licensing("rust-project-licensing"),
        cmd_init_config_file("init-config-file"),
        cmd_init_rust_project("init-rust-project"),
        cmd_distribution_info("python-distribution-info"),
        cmd_distribution_extract("python-distribution-extract"),
        cmd_distribution_licenses("python-distribution-licenses"),
        cmd_run_build_script("run-build-script"),
        cmd_generate_embedding_artifacts("generate-python-embedding-artifacts"),
    ];

    aliases
        .into_iter()
        .fold(app, |app, command| app.subcommand(command.hide(true)))
}

/// Map a `(command, subcommand)` pair onto the single name dispatched on.
///
/// `pyoxidizer dist info` and `pyoxidizer python-distribution-info` resolve to
/// the same canonical name and therefore run identical code.
fn canonical_command<'a>(name: &'a str, args: &'a ArgMatches) -> (&'a str, &'a ArgMatches) {
    fn leaf<'b>(
        group: &str,
        name: &str,
        args: &'b ArgMatches,
        mapping: &[(&str, &'static str)],
    ) -> Option<(&'static str, &'b ArgMatches)> {
        if group != name {
            return None;
        }

        let (sub, sub_args) = args.subcommand()?;

        mapping
            .iter()
            .find(|(key, _)| *key == sub)
            .map(|(_, canonical)| (*canonical, sub_args))
    }

    leaf(
        "dist",
        name,
        args,
        &[
            ("info", "python-distribution-info"),
            ("extract", "python-distribution-extract"),
            ("licenses", "python-distribution-licenses"),
        ],
    )
    .or_else(|| {
        leaf(
            "init",
            name,
            args,
            &[
                ("config", "init-config-file"),
                ("rust-project", "init-rust-project"),
            ],
        )
    })
    .or_else(|| {
        leaf(
            "inspect",
            name,
            args,
            &[
                ("binary", "analyze"),
                ("resources", "find-resources"),
                ("targets", "list-targets"),
                ("licensing", "rust-project-licensing"),
            ],
        )
    })
    .or_else(|| {
        leaf(
            "internal",
            name,
            args,
            &[
                ("build-script", "run-build-script"),
                ("embedding-artifacts", "generate-python-embedding-artifacts"),
            ],
        )
    })
    .unwrap_or((name, args))
}

pub fn run_cli() -> Result<()> {
    let mut env = crate::environment::Environment::new()?;

    let version = env.pyoxidizer_source.version_long();

    let app = Command::new("PyOxidizer")
        .version(PYOXIDIZER_VERSION)
        .long_version(version)
        .author("Gregory Szorc <gregory.szorc@gmail.com>")
        .long_about("Build and distribute Python applications")
        .arg_required_else_help(true)
        .infer_subcommands(true)
        .after_help(AFTER_HELP)
        .arg(
            Arg::new("system_rust")
                .long("system-rust")
                .env("PYOXIDIZER_SYSTEM_RUST")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Use a system install of Rust instead of a self-managed Rust installation"),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                .long("verbose")
                .global(true)
                .action(ArgAction::Count)
                .help("Increase logging verbosity. Can be specified multiple times"),
        )
        .arg(
            Arg::new("quiet")
                .short('q')
                .long("quiet")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Suppress all logging. Takes precedence over --verbose"),
        );

    // Top level commands.
    let app = app
        .subcommand(cmd_build())
        .subcommand(cmd_run())
        .subcommand(cmd_python())
        .subcommand(cmd_pyoxy())
        .subcommand(cmd_package())
        .subcommand(cmd_cache_clear("cache-clear"))
        .subcommand(cmd_completions());

    // Namespaces, then the flat spellings they replaced.
    let app = app
        .subcommand(group_dist())
        .subcommand(group_init())
        .subcommand(group_inspect())
        .subcommand(group_internal());

    let app = add_flat_aliases(app);

    let mut completions_app = app.clone();

    let matches = app.get_matches();

    let quiet = matches.get_flag("quiet");
    let verbose = !quiet && matches.get_count("verbose") > 0;

    let log_level = if quiet {
        log::LevelFilter::Warn
    } else {
        match matches.get_count("verbose") {
            0 => log::LevelFilter::Warn,
            1 => log::LevelFilter::Info,
            2 => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        }
    };

    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(log_level.as_str()),
    );

    builder
        .format_timestamp(None)
        .format_level(false)
        .format_target(false);

    builder.init();

    if matches.get_flag("system_rust") {
        env.unmanage_rust().context("unmanaging Rust")?;
    }

    let (name, name_args) = matches
        .subcommand()
        .ok_or_else(|| anyhow!("invalid sub-command"))?;

    let (command, args) = canonical_command(name, name_args);

    with_shell(|log| {
        log.set_verbosity(
            if quiet {
                Verbosity::Quiet
            } else if !verbose {
                Verbosity::Normal
            } else {
                Verbosity::Verbose
            }
        );
    });

    match command {
        "analyze" => {
            let path = args.get_one::<PathBuf>("path").unwrap();

            tugger_binary_analysis::analyze_file(path.clone());

            Ok(())
        }

        "build" => {
            let starlark_vars = starlark_vars(args)?;
            let release = args.get_flag("release");
            let target_triple = args.get_one::<String>("target_triple");
            let path = args.get_one::<PathBuf>("path").unwrap();

            // Accept both `build exe install` and `build --target exe`.
            let positional = args
                .get_many::<String>("positional_targets")
                .unwrap_or_default()
                .cloned()
                .collect::<Vec<_>>();
            let options = args
                .get_many::<String>("targets")
                .unwrap_or_default()
                .cloned()
                .collect::<Vec<_>>();

            let resolve_targets = if positional.is_empty() && options.is_empty() {
                None
            } else {
                Some(positional.into_iter().chain(options).collect::<Vec<_>>())
            };

            projectmgmt::build(
                &env,
                path,
                target_triple.map(|x| x.as_str()),
                resolve_targets,
                starlark_vars,
                release,
                verbose,
            )
        }

        "cache-clear" => projectmgmt::cache_clear(&env),

        "find-resources" => {
            let path = args.get_one::<PathBuf>("path");
            let distributions_dir = args.get_one::<PathBuf>("distributions_dir");
            let scan_distribution = args.get_flag("scan_distribution");
            let target_triple = args
                .get_one::<String>("target_triple")
                .expect("target_triple should have default");
            let classify_files = !args.get_flag("no_classify_files");
            let emit_files = !args.get_flag("no_emit_files");

            projectmgmt::find_resources(
                &env,
                path.map(|x| x.as_path()),
                distributions_dir.map(|x| x.as_path()),
                scan_distribution,
                target_triple,
                classify_files,
                emit_files,
            )
        }

        "generate-python-embedding-artifacts" => {
            let target_triple = args
                .get_one::<String>("target_triple")
                .expect("target_triple should have default");
            let flavor = args
                .get_one::<String>("flavor")
                .expect("flavor should have default");
            let python_version = args.get_one::<String>("python_version");
            let dest_path = args
                .get_one::<PathBuf>("dest_path")
                .expect("dest_path should be required");

            projectmgmt::generate_python_embedding_artifacts(
                &env,
                target_triple,
                flavor,
                python_version.map(|x| x.as_str()),
                dest_path,
            )
        }

        "init-config-file" => {
            let code = args.get_one::<String>("python-code");
            let pip_install = args
                .get_many::<String>("pip-install")
                .unwrap_or_default()
                .map(|x| x.as_str())
                .collect::<Vec<_>>();
            let path = args.get_one::<PathBuf>("path").unwrap();

            projectmgmt::init_config_file(
                &env.pyoxidizer_source,
                path,
                code.map(|x| x.as_str()),
                &pip_install,
            )
        }

        "list-targets" => {
            let path = args.get_one::<PathBuf>("path").unwrap();

            projectmgmt::list_targets(&env, path)
        }

        "init-rust-project" => {
            let path = args.get_one::<PathBuf>("path").unwrap();

            projectmgmt::init_rust_project(&env, path)
        }

        "python-distribution-extract" => {
            let download_default = args.get_flag("download_default");
            let archive_path = args.get_one::<PathBuf>("archive_path");
            let dest_path = args.get_one::<PathBuf>("dest_path").unwrap();

            projectmgmt::python_distribution_extract(
                download_default,
                archive_path.map(|x| x.as_path()),
                dest_path,
            )
        }

        "python-distribution-info" => {
            let dist_path = args.get_one::<PathBuf>("path").unwrap();

            projectmgmt::python_distribution_info(&env, dist_path)
        }

        "python-distribution-licenses" => {
            let path = args.get_one::<PathBuf>("path").unwrap();

            projectmgmt::python_distribution_licenses(&env, path)
        }

        "run-build-script" => {
            let starlark_vars = starlark_vars(args)?;
            let build_script = args.get_one::<String>("build_script_name").unwrap();
            let target = args.get_one::<String>("target");

            project_building::run_from_build(
                &env,
                build_script,
                target.map(|x| x.as_str()),
                starlark_vars,
            )
        }

        "run" => {
            let starlark_vars = starlark_vars(args)?;
            let target_triple = args.get_one::<String>("target_triple");
            let release = args.get_flag("release");
            let path = args.get_one::<PathBuf>("path").unwrap();
            let target = args.get_one::<String>("target");
            let extra = args
                .get_many::<String>("extra")
                .unwrap_or_default()
                .map(|x| x.as_str())
                .collect::<Vec<_>>();

            projectmgmt::run(
                &env,
                path,
                target_triple.map(|x| x.as_str()),
                release,
                target.map(|x| x.as_str()),
                starlark_vars,
                &extra,
                verbose,
            )
        }

        "python" => {
            let target_triple = args
                .get_one::<String>("target_triple")
                .expect("target_triple should have default");
            let flavor = args
                .get_one::<String>("flavor")
                .expect("flavor should have default");
            let python_version = args.get_one::<String>("python_version");
            let dest_path = args.get_one::<PathBuf>("dest_path").unwrap();
            let name = args.get_one::<String>("name").unwrap();
            let release = args.get_flag("release");

            projectmgmt::build_standalone_python(
                &env,
                name,
                dest_path,
                target_triple,
                flavor,
                python_version.map(|x| x.as_str()),
                release,
                verbose,
            )
        }

        "pyoxy" => {
            let starlark_vars = starlark_vars(args)?;
            let target_triple = args.get_one::<String>("target_triple");
            let release = args.get_flag("release");
            let path = args.get_one::<PathBuf>("path").unwrap();
            let dest_path = args.get_one::<PathBuf>("dest_path").unwrap();
            let name = args.get_one::<String>("name").unwrap();
            let split_resources = args.get_flag("split_resources");

            projectmgmt::build_pyoxy_runner(
                &env,
                path,
                dest_path,
                name,
                target_triple.map(|x| x.as_str()),
                starlark_vars,
                release,
                split_resources,
                verbose,
            )
        }

        "package" => {
            let target_triple = args
                .get_one::<String>("target_triple")
                .expect("target_triple should have default");
            let flavor = args
                .get_one::<String>("flavor")
                .expect("flavor should have default");
            let python_version = args.get_one::<String>("python_version");
            let dest_path = args.get_one::<PathBuf>("dest_path").unwrap();
            let requirements = args
                .get_many::<String>("requirements")
                .unwrap_or_default()
                .cloned()
                .collect::<Vec<_>>();

            projectmgmt::python_package_generate_packed_resources(
                &env,
                dest_path,
                &requirements,
                target_triple,
                flavor,
                python_version.map(|x| x.as_str()),
                verbose,
            )
        }

        "rust-project-licensing" => {
            let project_path = args.get_one::<PathBuf>("path").expect("path is required");
            let all_features = args.get_flag("all_features");
            let target_triple = args.get_one::<String>("target_triple");
            let unified_license = args.get_flag("unified_license");

            projectmgmt::rust_project_licensing(
                &env,
                project_path,
                all_features,
                target_triple.map(|x| x.as_str()),
                unified_license,
            )
        }

        "completions" => {
            let shell = match args
                .get_one::<String>("shell")
                .expect("shell is required")
                .as_str()
            {
                "bash" => Shell::Bash,
                "elvish" => Shell::Elvish,
                "fish" => Shell::Fish,
                "powershell" => Shell::PowerShell,
                "zsh" => Shell::Zsh,
                value => {
                    return Err(anyhow!(
                        "unsupported shell '{}'; supported are bash, elvish, fish, powershell and zsh",
                        value
                    ));
                }
            };

            clap_complete::generate(
                shell,
                &mut completions_app,
                "pyoxidizer",
                &mut std::io::stdout(),
            );

            Ok(())
        }

        _ => Err(anyhow!("invalid sub-command")),
    }
}
