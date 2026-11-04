// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Manage PyOxidizer projects.

use {
    crate::{
        environment::{Environment, PyOxidizerSource, canonicalize_path, default_target_triple},
        licensing::{licenses_from_cargo_manifest, log_licensing_info},
        project_building::find_pyoxidizer_config_file_env,
        project_layout::{
            initialize_project, render_standalone_python_config, write_new_pyoxidizer_config_file,
        },
        py_packaging::{
            distribution::{
                BinaryLibpythonLinkMode, DistributionCache, DistributionFlavor, PythonDistribution,
                default_distribution_location, resolve_distribution,
                resolve_python_distribution_archive,
            },
            packaging_tool::pip_download_wheel_resources,
            standalone_distribution::StandaloneDistribution,
        },
        python_distributions::PYTHON_DISTRIBUTIONS,
        shell::with_shell,
        starlark::eval::EvaluationContextBuilder,
    },
    anyhow::{Context, Result, anyhow},
    duct::cmd,
    log::warn,
    python_packaging::licensing::LicenseFlavor,
    python_packaging::{
        bytecode::BytecodeCompiler,
        filesystem_scanning::find_python_resources,
        interpreter::{MemoryAllocatorBackend, PythonInterpreterProfile},
        location::{AbstractResourceLocation, ConcreteResourceLocation},
        policy::ExtensionModuleFilter,
        resource::PythonResource,
        resource_collection::PythonResourceCollector,
        wheel::WheelArchive,
    },
    simple_file_manifest::{FileData, FileManifest},
    std::{
        collections::HashMap,
        fs::create_dir_all,
        io::{BufRead, BufReader, Cursor, Read, Write},
        path::{Path, PathBuf},
    },
};

/// Attempt to resolve the default Rust target for a build.
pub fn default_target() -> Result<String> {
    // TODO derive these more intelligently.
    if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            Ok("aarch64-unknown-linux-gnu".to_string())
        } else {
            Ok("x86_64-unknown-linux-gnu".to_string())
        }
    } else if cfg!(target_os = "windows") {
        Ok("x86_64-pc-windows-msvc".to_string())
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            Ok("aarch64-apple-darwin".to_string())
        } else {
            Ok("x86_64-apple-darwin".to_string())
        }
    } else {
        Err(anyhow!("unable to resolve target"))
    }
}

pub fn resolve_target(target: Option<&str>) -> Result<String> {
    if let Some(s) = target {
        Ok(s.to_string())
    } else {
        default_target()
    }
}

pub fn list_targets(env: &Environment, project_path: &Path) -> Result<()> {
    let config_path = find_pyoxidizer_config_file_env(project_path).ok_or_else(|| {
        anyhow!(
            "unable to find PyOxidizer config file at {}",
            project_path.display()
        )
    })?;

    let target_triple = default_target()?;

    let mut context = EvaluationContextBuilder::new(env, config_path.clone(), target_triple)
        .resolve_targets(vec![])
        .into_context()?;

    context.evaluate_file(&config_path)?;

    if context.default_target()?.is_none() {
        println!("(no targets defined)");
        return Ok(());
    }

    for target in context.target_names()? {
        let prefix = if Some(target.clone()) == context.default_target()? {
            "*"
        } else {
            ""
        };
        println!("{}{}", prefix, target);
    }

    Ok(())
}

/// Build a PyOxidizer enabled project.
///
/// This is a glorified wrapper around `cargo build`. Our goal is to get the
/// output from repackaging to give the user something for debugging.
#[allow(clippy::too_many_arguments)]
pub fn build(
    env: &Environment,
    project_path: &Path,
    target_triple: Option<&str>,
    resolve_targets: Option<Vec<String>>,
    extra_vars: HashMap<String, Option<String>>,
    release: bool,
    verbose: bool,
) -> Result<()> {
    let config_path = find_pyoxidizer_config_file_env(project_path).ok_or_else(|| {
        anyhow!(
            "unable to find PyOxidizer config file at {}",
            project_path.display()
        )
    })?;
    let target_triple = resolve_target(target_triple)?;

    let mut context = EvaluationContextBuilder::new(env, config_path.clone(), target_triple)
        .extra_vars(extra_vars)
        .release(release)
        .verbose(verbose)
        .resolve_targets_optional(resolve_targets)
        .into_context()?;

    context.evaluate_file(&config_path)?;

    for target in context.targets_to_resolve()? {
        context.build_resolved_target(&target)?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    env: &Environment,
    project_path: &Path,
    target_triple: Option<&str>,
    release: bool,
    target: Option<&str>,
    extra_vars: HashMap<String, Option<String>>,
    _extra_args: &[&str],
    verbose: bool,
) -> Result<()> {
    let config_path = find_pyoxidizer_config_file_env(project_path).ok_or_else(|| {
        anyhow!(
            "unable to find PyOxidizer config file at {}",
            project_path.display()
        )
    })?;
    let target_triple = resolve_target(target_triple)?;

    let mut context = EvaluationContextBuilder::new(env, config_path.clone(), target_triple)
        .extra_vars(extra_vars)
        .release(release)
        .verbose(verbose)
        .resolve_target_optional(target)
        .into_context()?;

    context.evaluate_file(&config_path)?;

    context.run_target(target)
}

pub fn cache_clear(env: &Environment) -> Result<()> {
    let cache_dir = env.cache_dir();

    println!("removing {}", cache_dir.display());
    remove_dir_all::remove_dir_all(cache_dir)?;

    Ok(())
}

/// Find resources given a source path.
pub fn find_resources(
    env: &Environment,
    path: Option<&Path>,
    distributions_dir: Option<&Path>,
    scan_distribution: bool,
    target_triple: &str,
    classify_files: bool,
    emit_files: bool,
) -> Result<()> {
    let distribution_location =
        default_distribution_location(&DistributionFlavor::Standalone, target_triple, None)?;

    let extract_path = if let Some(path) = distributions_dir {
        path.to_owned()
    } else {
        env.python_distributions_dir()
    };

    let dist = resolve_distribution(&distribution_location, &extract_path)?;

    if scan_distribution {
        println!("scanning distribution");
        for resource in dist.python_resources() {
            print_resource(&resource);
        }
    } else if let Some(path) = path {
        if path.is_dir() {
            println!("scanning directory {}", path.display());
            for resource in find_python_resources(
                path,
                dist.cache_tag(),
                &dist.python_module_suffixes()?,
                emit_files,
                classify_files,
            )? {
                print_resource(&resource?);
            }
        } else if path.is_file() {
            if let Some(extension) = path.extension() {
                if extension.to_string_lossy() == "whl" {
                    println!("parsing {} as a wheel archive", path.display());
                    let wheel = WheelArchive::from_path(path)?;

                    for resource in wheel.python_resources(
                        dist.cache_tag(),
                        &dist.python_module_suffixes()?,
                        emit_files,
                        classify_files,
                    )? {
                        print_resource(&resource)
                    }

                    return Ok(());
                }
            }

            println!("do not know how to find resources in {}", path.display());
        } else {
            println!("do not know how to find resources in {}", path.display());
        }
    } else {
        println!("do not know what to scan");
    }

    Ok(())
}

fn print_resource(r: &PythonResource) {
    match r {
        PythonResource::ModuleSource(m) => println!(
            "PythonModuleSource {{ name: {}, is_package: {}, is_stdlib: {}, is_test: {} }}",
            m.name, m.is_package, m.is_stdlib, m.is_test
        ),
        PythonResource::ModuleBytecode(m) => println!(
            "PythonModuleBytecode {{ name: {}, is_package: {}, is_stdlib: {}, is_test: {}, bytecode_level: {} }}",
            m.name, m.is_package, m.is_stdlib, m.is_test, i32::from(m.optimize_level)
        ),
        PythonResource::ModuleBytecodeRequest(_) => println!(
            "PythonModuleBytecodeRequest {{ you should never see this }}"
        ),
        PythonResource::PackageResource(r) => println!(
            "PythonPackageResource {{ package: {}, name: {}, is_stdlib: {}, is_test: {} }}", r.leaf_package, r.relative_name, r.is_stdlib, r.is_test
        ),
        PythonResource::PackageDistributionResource(r) => println!(
            "PythonPackageDistributionResource {{ package: {}, version: {}, name: {} }}", r.package, r.version, r.name
        ),
        PythonResource::ExtensionModule(em) => {
            println!(
                "PythonExtensionModule {{"
            );
            println!("    name: {}", em.name);
            println!("    is_builtin: {}", em.builtin_default);
            println!("    has_shared_library: {}", em.shared_library.is_some());
            println!("    has_object_files: {}", !em.object_file_data.is_empty());
            println!("    link_libraries: {:?}", em.link_libraries);
            println!("}}");
        },
        PythonResource::EggFile(e) => println!(
            "PythonEggFile {{ path: {} }}", match &e.data {
                FileData::Path(p) => p.display().to_string(),
                FileData::Memory(_) => "memory".to_string(),
            }
        ),
        PythonResource::PathExtension(_pe) => println!(
            "PythonPathExtension",
        ),
        PythonResource::File(f) => println!(
            "File {{ path: {}, is_executable: {} }}", f.path().display(), f.entry().is_executable()
        ),
    }
}

/// Initialize a PyOxidizer configuration file in a given directory.
pub fn init_config_file(
    source: &PyOxidizerSource,
    project_dir: &Path,
    code: Option<&str>,
    pip_install: &[&str],
) -> Result<()> {
    if project_dir.exists() && !project_dir.is_dir() {
        return Err(anyhow!(
            "existing path must be a directory: {}",
            project_dir.display()
        ));
    }

    if !project_dir.exists() {
        create_dir_all(project_dir)?;
    }

    // Derive the project name from the directory. `Path::iter()` on relative
    // paths such as `.` yields that literal component, which would become the
    // name of the produced executable. Resolve to an absolute path first.
    let name = project_dir
        .canonicalize()
        .context("canonicalizing project directory")?
        .file_name()
        .ok_or_else(|| {
            anyhow!(
                "could not determine project name from {}",
                project_dir.display()
            )
        })?
        .to_str()
        .ok_or_else(|| anyhow!("project directory is not valid UTF-8"))?
        .to_string();

    write_new_pyoxidizer_config_file(source, project_dir, &name, code, pip_install)?;

    println!();
    println!("A new PyOxidizer configuration file has been created.");
    println!("This configuration file can be used by various `pyoxidizer`");
    println!("commands");
    println!();
    println!("For example, to build and run the default Python application:");
    println!();
    println!("  $ cd {}", project_dir.display());
    println!("  $ pyoxidizer run");
    println!();
    println!("The default configuration is to invoke a Python REPL. You can");
    println!("edit the configuration file to change behavior.");

    Ok(())
}

/// Initialize a new Rust project with PyOxidizer support.
pub fn init_rust_project(env: &Environment, project_path: &Path) -> Result<()> {
    let cargo_exe = env
        .ensure_rust_toolchain(None)
        .context("resolving Rust environment")?
        .cargo_exe;

    initialize_project(
        &env.pyoxidizer_source,
        project_path,
        &cargo_exe,
        None,
        &[],
        "console",
    )?;
    println!();
    println!(
        "A new Rust binary application has been created in {}",
        project_path.display()
    );
    print!(
        r#"
This application can be built most easily by doing the following:

  $ cd {project_path}
  $ pyoxidizer run

Note however that this will bypass all the Rust code in the project
folder, and build the project as if you had only created a pyoxidizer.bzl
file. Building from Rust is more involved, and requires multiple steps.
Please see the "PyOxidizer Rust Projects" section of the manual for more
information.

The default configuration is to invoke a Python REPL. You can
edit the various pyoxidizer.*.bzl config files or the main.rs
file to change behavior. The application will need to be rebuilt
for configuration changes to take effect.
"#,
        project_path = project_path.display()
    );

    Ok(())
}

/// Build a self-contained Python interpreter executable.
///
/// This backs `pyoxidizer python`. A purpose-built configuration file is
/// written to a temporary directory, its `install` target is built, and the
/// resulting executable - plus the licensing documentation required to
/// redistribute it - is copied to `dest_path`.
#[allow(clippy::too_many_arguments)]
pub fn build_standalone_python(
    env: &Environment,
    name: &str,
    dest_path: &Path,
    target_triple: &str,
    flavor: &str,
    python_version: Option<&str>,
    release: bool,
    verbose: bool,
) -> Result<()> {
    let config_source = render_standalone_python_config(name, flavor, python_version)
        .context("rendering standalone Python interpreter configuration")?;

    let temp_dir = env.temporary_directory("pyoxidizer-python")?;
    let project_path = temp_dir.path();

    let config_path = project_path.join("pyoxidizer.bzl");
    std::fs::write(&config_path, config_source.as_bytes())
        .context("writing generated configuration file")?;

    // Keep intermediate artifacts within the temporary directory so this
    // doesn't litter the user's current directory with a build/ tree.
    let mut extra_vars = HashMap::new();
    extra_vars.insert(
        "BUILD_PATH".to_string(),
        Some(project_path.join("build").display().to_string()),
    );

    let mut context =
        EvaluationContextBuilder::new(env, config_path.clone(), target_triple.to_string())
            .extra_vars(extra_vars)
            .release(release)
            .verbose(verbose)
            .resolve_targets(vec!["install".to_string()])
            .into_context()
            .context("creating evaluation context")?;

    context
        .evaluate_file(&config_path)
        .context("evaluating generated configuration file")?;

    let resolved = context
        .build_resolved_target("install")
        .context("building install target")?;

    // Windows executables need the extension to be runnable.
    let exe_file_name = if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    };

    let source_exe = resolved.output_path.join(&exe_file_name);
    if !source_exe.exists() {
        return Err(anyhow!(
            "build did not produce an executable at {}",
            source_exe.display()
        ));
    }

    create_dir_all(dest_path)
        .with_context(|| format!("creating destination directory {}", dest_path.display()))?;

    let dest_exe = dest_path.join(&exe_file_name);
    std::fs::copy(&source_exe, &dest_exe)
        .with_context(|| format!("copying executable to {}", dest_exe.display()))?;

    // Licensing documentation is emitted alongside the built executable and is
    // required when the binary is redistributed.
    for file_name in ["COPYING.txt", "COPYING.md"] {
        let source = resolved.output_path.join(file_name);
        if source.exists() {
            std::fs::copy(&source, dest_path.join(file_name)).with_context(|| {
                format!("copying {} to {}", file_name, dest_path.display())
            })?;
        }
    }

    // The temporary directory is deleted when `temp_dir` is dropped, so
    // everything the user needs has to have been copied out above.
    println!("wrote {}", dest_exe.display());

    Ok(())
}

/// Write one packed resources file per Python package.
///
/// Each requirement is resolved with `pip download` (wheels only, including
/// transitive dependencies); every resolved wheel is classified and serialized
/// to `<name>.packed-resources` in `dest_path`, one file per pip package.
/// Extensions modules and other resources that cannot be loaded from memory
/// are skipped with a warning.
pub fn python_package_generate_packed_resources(
    env: &Environment,
    dest_path: &Path,
    requirements: &[String],
    target_triple: &str,
    flavor: &str,
    python_version: Option<&str>,
    verbose: bool,
) -> Result<()> {
    if requirements.is_empty() {
        return Err(anyhow!("no requirements to package"));
    }

    let distribution_location = default_distribution_location(
        &DistributionFlavor::try_from(flavor).map_err(|e| anyhow!("invalid --flavor: {}", e))?,
        target_triple,
        python_version,
    )?;

    // Resolution is shared with `pyoxidizer dist extract`, including its
    // cache handling.
    let cache_dir = env.python_distributions_dir();

    let dist = resolve_distribution(&distribution_location, &cache_dir)
        .context("resolving Python distribution")?;

    let mut policy = dist.create_packaging_policy()?;
    policy.set_extension_module_filter(ExtensionModuleFilter::All);
    policy.set_include_distribution_sources(true);
    policy.set_include_distribution_resources(true);
    policy.set_resources_location(ConcreteResourceLocation::InMemory);
    // Bytecode for every level so imports succeed under -O and -OO, matching
    // the policy of `pyoxidizer python`.
    policy.set_bytecode_optimize_level_zero(true);
    policy.set_bytecode_optimize_level_one(true);
    policy.set_bytecode_optimize_level_two(true);

    let wheels = pip_download_wheel_resources(
        env,
        dist.as_ref(),
        dist.as_ref(),
        &policy,
        verbose,
        requirements,
    )
    .context("downloading Python packages with pip")?;

    create_dir_all(dest_path)
        .with_context(|| format!("creating destination directory {}", dest_path.display()))?;

    // Bytecode compilation is driven by the distribution's own Python.
    let compiler_temp = env.temporary_directory("python-package-bytecode")?;
    let mut compiler = BytecodeCompiler::new(dist.python_exe_path(), compiler_temp.path())?;

    for (name, resources) in wheels {
        let mut collector = PythonResourceCollector::new(
            vec![AbstractResourceLocation::InMemory],
            // Extension modules are never written; keep the policy closed so
            // mistakes fail instead of silently embedding .so data.
            vec![],
            false,
            false,
        );

        for resource in &resources {
            match resource {
                PythonResource::ModuleSource(r) => {
                    collector.add_python_module_source(r, &ConcreteResourceLocation::InMemory)?;
                }
                PythonResource::PackageResource(r) => {
                    collector
                        .add_python_package_resource(r, &ConcreteResourceLocation::InMemory)?;
                }
                PythonResource::PackageDistributionResource(r) => {
                    collector.add_python_package_distribution_resource(
                        r,
                        &ConcreteResourceLocation::InMemory,
                    )?;
                }
                // Resources that cannot be loaded from memory would make the
                // packed resources file incomplete or unloadable. Skip them so
                // e.g. requests packages despite its C accelerator wheels,
                // mirroring the bake path in pyoxy/pyoxidizer.bzl.
                PythonResource::ExtensionModule(_) | PythonResource::File(_) => {
                    let kind = match resource {
                        PythonResource::ExtensionModule(_) => "C extension module",
                        _ => "untyped file",
                    };
                    warn!(
                        "skipping {} from {} because it cannot be loaded from memory",
                        kind, name
                    );
                }
                _ => {
                    return Err(anyhow!(
                        "{} contains a resource that cannot be written to a single packed resources file: {:?}",
                        name,
                        resource
                    ));
                }
            }
        }

        let compiled = collector
            .compile_resources(&mut compiler)
            .with_context(|| format!("compiling resources of {}", name))?;

        // In-memory resources produce no extra file writes; wheels that do
        // would make the per-package file incomplete, so fail loudly.
        if !compiled.extra_files.is_empty() {
            return Err(anyhow!(
                "{} produced extra files that cannot be part of its packed resources file",
                name
            ));
        }

        let dest_file = dest_path.join(format!("{}.packed-resources", name));

        let mut fh = std::fs::File::create(&dest_file)
            .with_context(|| format!("opening {} for writing", dest_file.display()))?;
        let mut writer = std::io::BufWriter::new(&mut fh);

        compiled
            .write_packed_resources(&mut writer)
            .with_context(|| format!("writing {}", dest_file.display()))?;

        writer.flush()?;
        drop(writer);

        println!("wrote {}", dest_file.display());
    }

    compiler_temp
        .close()
        .context("closing temporary directory")?;

    Ok(())
}

pub fn build_pyoxy_runner(
    env: &Environment,
    project_path: &Path,
    dest_path: &Path,
    exe_name: &str,
    target_triple: Option<&str>,
    extra_vars: HashMap<String, Option<String>>,
    release: bool,
    verbose: bool,
) -> Result<()> {
    // First stage: evaluate the project's `resources` target, which writes the
    // embedding artifacts - pyo3 config, packed resources, licensing - into
    // the build directory.
    let mut vars = extra_vars;

    // The embedding artifacts for Python-linked executables always include the
    // stdlib resources. Pointing BUILD_PATH at a project-local directory keeps
    // them grouped with the binary that consumes them, mirroring the way
    // `pyoxidizer python` builds. The path must be absolute: Starlark resolves
    // a relative BUILD_PATH against its own CWD, which is the project
    // directory, not the process's.
    let project_path = std::fs::canonicalize(project_path)
        .with_context(|| format!("resolving {}", project_path.display()))?;

    // Capture the effective build path before `vars` is consumed by build().
    vars.entry("BUILD_PATH".to_string())
        .or_insert_with(|| Some(project_path.join("build").display().to_string()));
    let build_dir = vars
        .get("BUILD_PATH")
        .cloned()
        .and_then(|x| x)
        .filter(|x| !x.is_empty())
        .map(|x| {
            std::env::current_dir()
                .map(|cwd| cwd.join(&x))
                .unwrap_or_else(|_| PathBuf::from(&x))
        })
        .unwrap_or_else(|| project_path.join("build"));

    build(
        env,
        &project_path,
        target_triple,
        Some(vec!["resources".to_string()]),
        vars,
        release,
        verbose,
    )
    .context("building Python embedding artifacts")?;

    // Second stage: build the crate's binary against those artifacts.
    //
    // This runs the same two steps the CI scripts perform by hand: evaluate
    // the embedding artifacts, then build with PYO3_CONFIG_FILE pointing at
    // them. cargo is taken from PATH, like it is elsewhere in this repository.
    let target_triple = resolve_target(target_triple)?;

    let profile = if release { "release" } else { "debug" };

    // The artifacts of a target whose build path is <BUILD_PATH> land in
    // <BUILD_PATH>/<triple>/<profile>/resources/pyo3-build-config-file.txt.
    // Look for exactly that, then fall back to a bounded search in case the
    // target layout shifts between pyoxidizer versions.
    let artifacts_dir = find_pyo3_config_dir(&build_dir);
    let artifacts_dir = match artifacts_dir {
        Some(dir) => dir,
        None => {
            return Err(anyhow!(
                "the resources target of {} did not write a pyo3-build-config-file.txt under {}",
                project_path.display(),
                build_dir.display()
            ));
        }
    };

    let pyo3_config = artifacts_dir
        .join("resources")
        .join("pyo3-build-config-file.txt");

    let bin_name = "pyoxy";

    // Cargo will rebuild when PYO3_CONFIG_FILE changes thanks to
    // `cargo:rerun-if-env-changed` in the crate's build script, so repeated
    // invocations with a different --python-version do produce a rebuild.
    // --target-dir pins the output location: the crate may be a workspace
    // member, in which case cargo's default target directory is the
    // workspace's, not the project's.
    let exe_path = project_path
        .join("target")
        .join(&target_triple)
        .join(profile)
        .join(exe_name_file_with_suffix(bin_name, &target_triple));

    let mut cmd_args = vec![
        "build".to_string(),
        "--target".to_string(),
        target_triple.to_string(),
        "--bin".to_string(),
        bin_name.to_string(),
        "--target-dir".to_string(),
        project_path.join("target").display().to_string(),
    ];

    if with_shell(|log| log.out_supports_color()) {
        cmd_args.push("--color=always".to_string());
    }

    if release {
        cmd_args.push("--release".to_string());
    }

    let mut env_vars: HashMap<String, String> = std::env::vars().collect();
    env_vars.insert(
        "PYO3_CONFIG_FILE".to_string(),
        pyo3_config.display().to_string(),
    );

    let command = cmd("cargo", &cmd_args)
        .full_env(env_vars)
        .dir(project_path)
        .stderr_to_stdout()
        .unchecked()
        .reader()
        .context("invoking cargo command")?;

    {
        let reader = BufReader::new(&command);
        for line in reader.lines() {
            let _ = log::warn!("{}", line.context("reading cargo output")?);
        }
    }

    let output = command
        .try_wait()
        .context("waiting on cargo process")?
        .ok_or_else(|| anyhow!("unable to wait on command"))?;

    if !output.status.success() {
        return Err(anyhow!("cargo build failed"));
    }

    if !exe_path.exists() {
        return Err(anyhow!(
            "cargo build did not produce an executable at {}",
            exe_path.display()
        ));
    }

    create_dir_all(dest_path)
        .with_context(|| format!("creating destination directory {}", dest_path.display()))?;

    let dest_exe = dest_path.join(exe_name_file_with_suffix(&exe_name, &target_triple));

    std::fs::copy(&exe_path, &dest_exe)
        .with_context(|| format!("copying executable to {}", dest_exe.display()))?;

    println!("wrote {}", dest_exe.display());
    println!("artifacts: {}", artifacts_dir.display());

    Ok(())
}

/// Return the file name `exe_name` should have for `target_triple`.
fn exe_name_file_with_suffix(exe_name: &str, target_triple: &str) -> String {
    if target_triple.contains("windows") {
        format!("{}.exe", exe_name)
    } else {
        exe_name.to_string()
    }
}

/// Find the directory under `build_dir` containing a `resources/`
/// `pyo3-build-config-file.txt`, preferring depth over breadth.
///
/// The regular layout is `<build>/<triple>/<profile>/resources/`, but the
/// depth-limited walk keeps this working if the intermediate directory names
/// change. Returns None when nothing was found.
fn find_pyo3_config_dir(build_dir: &Path) -> Option<PathBuf> {
    const MAX_DEPTH: usize = 4;

    fn walk(dir: &Path, depth: usize, found: &mut Option<PathBuf>) -> bool {
        if found.is_some() {
            return true;
        }

        if depth > MAX_DEPTH {
            return false;
        }

        // Configuration directories are always resolved targets; profile dirs
        // sort deterministically and a single project has one artifacts dir.
        if dir.join("resources").join("pyo3-build-config-file.txt").exists() {
            *found = Some(dir.to_path_buf());
            return true;
        }

        let mut entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|x| x.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect::<Vec<_>>(),
            Err(_) => return false,
        };
        // Bubble the newest profile first, mirroring release selection after a
        // fresh run, but stay deterministic otherwise.
        entries.sort();
        entries.reverse();

        for entry in entries {
            if walk(&entry, depth + 1, found) {
                return true;
            }
        }

        false
    }

    let mut found: Option<PathBuf> = None;
    walk(build_dir, 0, &mut found);
    found
}

pub fn python_distribution_extract(
    download_default: bool,
    archive_path: Option<&Path>,
    dest_path: &Path,
) -> Result<()> {
    let dist_path = if let Some(path) = archive_path {
        path.to_path_buf()
    } else if download_default {
        let location = default_distribution_location(
            &DistributionFlavor::Standalone,
            default_target_triple(),
            None,
        )?;

        resolve_python_distribution_archive(&location, dest_path)?
    } else {
        return Err(anyhow!("do not know what distribution to operate on"));
    };

    let mut fh = std::fs::File::open(&dist_path)?;
    let mut data = Vec::new();
    fh.read_to_end(&mut data)?;
    let cursor = Cursor::new(data);
    let dctx = zstd::stream::Decoder::new(cursor)?;
    let mut tf = tar::Archive::new(dctx);

    println!("extracting archive to {}", dest_path.display());
    tf.unpack(dest_path)?;

    Ok(())
}

pub fn python_distribution_info(env: &Environment, dist_path: &Path) -> Result<()> {
    let fh = std::fs::File::open(dist_path)?;
    let reader = std::io::BufReader::new(fh);

    let temp_dir = env.temporary_directory("python-distribution")?;
    let temp_dir_path = temp_dir.path();

    let dist = StandaloneDistribution::from_tar_zst(reader, temp_dir_path)?;

    println!("High-Level Metadata");
    println!("===================");
    println!();
    println!("Target triple: {}", dist.target_triple);
    println!("Tag:           {}", dist.python_tag);
    println!("Platform tag:  {}", dist.python_platform_tag);
    println!("Version:       {}", dist.version);
    println!();

    println!("Extension Modules");
    println!("=================");
    for (name, ems) in dist.extension_modules {
        println!("{}", name);
        println!("{}", "-".repeat(name.len()));
        println!();

        for em in ems.iter() {
            println!("{}", em.variant.as_ref().unwrap());
            println!("{}", "^".repeat(em.variant.as_ref().unwrap().len()));
            println!();
            println!("Required: {}", em.required);
            println!("Built-in Default: {}", em.builtin_default);
            if let Some(component) = &em.license {
                println!(
                    "Licensing: {}",
                    match component.license() {
                        LicenseFlavor::Spdx(expression) => expression.to_string(),
                        LicenseFlavor::OtherExpression(expression) => expression.to_string(),
                        LicenseFlavor::PublicDomain => "public domain".to_string(),
                        LicenseFlavor::None => "none".to_string(),
                        LicenseFlavor::Unknown(terms) => terms.join(","),
                    }
                );
            }
            if !em.link_libraries.is_empty() {
                println!(
                    "Links: {}",
                    em.link_libraries
                        .iter()
                        .map(|l| l.name.clone())
                        .collect::<Vec<String>>()
                        .join(", ")
                );
            }

            println!();
        }
    }

    println!("Python Modules");
    println!("==============");
    println!();
    for name in dist.py_modules.keys() {
        println!("{}", name);
    }
    println!();

    println!("Python Resources");
    println!("================");
    println!();

    for (package, resources) in dist.resources {
        for name in resources.keys() {
            println!("[{}].{}", package, name);
        }
    }

    Ok(())
}

pub fn python_distribution_licenses(env: &Environment, path: &Path) -> Result<()> {
    let fh = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(fh);

    let temp_dir = env.temporary_directory("python-distribution")?;
    let temp_dir_path = temp_dir.path();

    let dist = StandaloneDistribution::from_tar_zst(reader, temp_dir_path)?;

    println!(
        "Python Distribution Licenses: {}",
        match dist.licenses {
            Some(licenses) => itertools::join(licenses, ", "),
            None => "NO LICENSE FOUND".to_string(),
        }
    );
    println!();
    println!("Extension Libraries and License Requirements");
    println!("============================================");
    println!();

    for (name, variants) in &dist.extension_modules {
        for variant in variants.iter() {
            if variant.link_libraries.is_empty() {
                continue;
            }

            let name = if variant.variant.as_ref().unwrap() == "default" {
                name.clone()
            } else {
                format!("{} ({})", name, variant.variant.as_ref().unwrap())
            };

            println!("{}", name);
            println!("{}", "-".repeat(name.len()));
            println!();

            for link in &variant.link_libraries {
                println!("Dependency: {}", &link.name);
                println!(
                    "Link Type: {}",
                    if link.system {
                        "system"
                    } else if link.framework {
                        "framework"
                    } else {
                        "library"
                    }
                );

                println!();
            }

            if let Some(component) = &variant.license {
                match component.license() {
                    LicenseFlavor::Spdx(expression) => {
                        println!("Licensing: Valid SPDX: {}", expression);
                    }
                    LicenseFlavor::OtherExpression(expression) => {
                        println!("Licensing: Invalid SPDX: {}", expression);
                    }
                    LicenseFlavor::PublicDomain => {
                        println!("Licensing: Public Domain");
                    }
                    LicenseFlavor::None => {
                        println!("Licensing: None defined");
                    }
                    LicenseFlavor::Unknown(terms) => {
                        println!("Licensing: {}", terms.join(", "));
                    }
                }
            } else {
                println!("Licensing: UNKNOWN");
            }

            println!();
        }
    }

    Ok(())
}

/// Generate artifacts for embedding Python in a binary.
pub fn generate_python_embedding_artifacts(
    env: &Environment,
    target_triple: &str,
    flavor: &str,
    python_version: Option<&str>,
    dest_path: &Path,
) -> Result<()> {
    let flavor = DistributionFlavor::try_from(flavor).map_err(|e| anyhow!("{}", e))?;

    std::fs::create_dir_all(dest_path)
        .with_context(|| format!("creating directory {}", dest_path.display()))?;

    let dest_path = canonicalize_path(dest_path).context("canonicalizing destination directory")?;

    let distribution_record = PYTHON_DISTRIBUTIONS
        .find_distribution(target_triple, &flavor, python_version)
        .ok_or_else(|| anyhow!("could not find Python distribution matching requirements"))?;

    let distribution_cache = DistributionCache::new(Some(&env.python_distributions_dir()));

    let dist = distribution_cache
        .resolve_distribution(&distribution_record.location, None)
        .context("resolving Python distribution")?;

    let host_dist = distribution_cache
        .host_distribution(Some(dist.python_major_minor_version().as_str()), None)
        .context("resolving host distribution")?;

    let policy = dist
        .create_packaging_policy()
        .context("creating packaging policy")?;

    let mut interpreter_config = dist
        .create_python_interpreter_config()
        .context("creating Python interpreter config")?;

    interpreter_config.config.profile = PythonInterpreterProfile::Python;
    interpreter_config.allocator_backend = MemoryAllocatorBackend::Default;

    let mut builder = dist.as_python_executable_builder(
        default_target_triple(),
        target_triple,
        "python",
        BinaryLibpythonLinkMode::Default,
        &policy,
        &interpreter_config,
        Some(host_dist.clone_trait()),
    )?;

    builder.set_tcl_files_path(Some("tcl".to_string()));

    builder
        .add_distribution_resources(None)
        .context("adding distribution resources")?;

    let embedded_context = builder
        .to_embedded_python_context(env, "1")
        .context("resolving embedded context")?;

    embedded_context
        .write_files(&dest_path)
        .context("writing embedded artifact files")?;

    embedded_context
        .extra_files
        .materialize_files(&dest_path)
        .context("writing extra files")?;

    // Write out a copy of the standard library.
    let mut m = FileManifest::default();
    for resource in find_python_resources(
        &dist.stdlib_path,
        dist.cache_tag(),
        &dist.python_module_suffixes()?,
        true,
        false,
    )? {
        if let PythonResource::File(file) = resource? {
            m.add_file_entry(file.path(), file.entry())?;
        } else {
            panic!("find_python_resources() should only emit File variant");
        }
    }

    m.materialize_files_with_replace(dest_path.join("stdlib"))
        .context("writing standard library")?;

    Ok(())
}

pub fn rust_project_licensing(
    env: &Environment,
    project_path: &Path,
    all_features: bool,
    target_triple: Option<&str>,
    unified_license: bool,
) -> Result<()> {
    let manifest_path = project_path.join("Cargo.toml");

    let toolchain = env
        .ensure_rust_toolchain(None)
        .context("resolving Rust toolchain")?;

    let licensing = licenses_from_cargo_manifest(
        &manifest_path,
        all_features,
        [],
        target_triple,
        &toolchain,
        true,
    )?;

    if unified_license {
        println!("{}", licensing.aggregate_license_document(true)?);
    } else {
        log_licensing_info(&licensing);
    }

    Ok(())
}
