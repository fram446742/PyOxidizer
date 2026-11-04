set dotenv-load

# Recipe arguments are available as positional arguments ($1, $2, ... "$@")
# in shebang scripts. This lets recipes like `pyoxy-dev` take arbitrary
# interpreter arguments without them being re-split by the shell through
# {{args}} interpolation.
set positional-arguments

default:
  cargo build

exe_suffix := if os() == "windows" { ".exe" } else { "" }

macosx_deployment_target := if os() == "macos" {
  if arch() == "arm" {
    "11.0"
  } else {
    "10.9"
  }
} else {
  ""
}

lipo_command := if os() == "macos" {
  "lipo"
} else {
  "llvm-lipo-14"
}

tar_command := if os() == "macos" {
  "gtar"
} else {
  "tar"
}

# Rust target triple for the machine running these recipes. Mirrors
# pyoxidizer's own default target resolution.
default_triple := if os() == "macos" {
  if arch() == "arm" { "aarch64-apple-darwin" } else { "x86_64-apple-darwin" }
} else if os() == "windows" {
  "x86_64-pc-windows-msvc"
} else {
  if arch() == "arm" { "aarch64-unknown-linux-gnu" } else { "x86_64-unknown-linux-gnu" }
}

# Build a self-contained Python interpreter into dist/python/.
#
# The result accepts the same arguments as CPython's `python` and needs no
# Python installation. Pass python_version to choose the interpreter version,
# e.g. `just python 3.14`.
python python_version="":
  #!/usr/bin/env bash
  set -eo pipefail

  args=()
  if [ -n "{{python_version}}" ]; then
    args+=(--python-version "{{python_version}}")
  fi

  cargo run --release --quiet --bin pyoxidizer -- python --dest dist/python "${args[@]}"
  ls -lh dist/python/

# Build the self-contained pyoxy runner into dist/pyoxy/.
#
# pyoxy is the shipped runner: it behaves like `python` by default, can run
# interpreter configurations declared in YAML, and can act as a script
# interpreter via shebang. The binary statically links a Python distribution
# and embeds its standard library, so it needs no Python installation.
#
# libraries is an optional space separated list of Python packages to bake
# into the single executable, e.g. `just pyoxy 3.11 "requests pyyaml"`.
# Instead of baking, `just pyoxy-package` produces one removable file per
# package; files in dist/pyoxy-modules/ are automatically copied next to
# the built binary, so both mechanisms compose. Override the Rust target
# triple with `just pyoxy <python-version> <libraries> <triple>`, the Rust
# optimization level with the fourth argument (3, the default, or 2, 1, s, z;
# `z` shrinks the Rust part by ~1 MB out of a ~150 MB binary since the
# embedded Python distribution dominates), and pass a non-empty fifth
# argument to ship the resources next to the executable instead of embedding
# them: `just pyoxy 3.11 "requests" x86_64-unknown-linux-gnu 3 split` writes
# a ~23 MB binary (libpython is still linked statically; the Rust code is
# ~3 MB) plus `pyoxy-resources.packed-resources`. Keep both files together;
# the runner reads the blob from its own directory.
pyoxy python_version="3.11" libraries="" triple=default_triple opt="3" split="":
  #!/usr/bin/env bash
  set -eo pipefail

  case "{{opt}}" in
    3|2|1|s|z) ;;
    *) echo "opt-level must be one of 3, 2, 1, s, z (got '{{opt}}')" >&2; exit 1 ;;
  esac
  # cargo reads this at build time; `pyoxidizer pyoxy` passes its whole
  # environment through to the cargo invocation it runs.
  export CARGO_PROFILE_RELEASE_OPT_LEVEL="{{opt}}"

  args=(--release --target-triple {{triple}} --var PYTHON_VERSION {{python_version}})
  if [ -n "{{libraries}}" ]; then
    args+=(--var REQUIREMENTS "{{libraries}}")
  fi
  if [ -n "{{split}}" ]; then
    args+=(--split-resources)
  else
    # Switching back from split mode must not leave a stale blob behind.
    rm -f dist/pyoxy/pyoxy-resources.packed-resources
  fi

  cargo run --release --quiet --bin pyoxidizer -- pyoxy \
    --path pyoxy --dest dist/pyoxy "${args[@]}"

  # Ship any per-package library files with the executable. Fresh copy so
  # packages removed from dist/pyoxy-modules do not linger.
  if [ -d dist/pyoxy-modules ]; then
    rm -rf dist/pyoxy/pyoxy-modules
    cp -r dist/pyoxy-modules dist/pyoxy/pyoxy-modules
    ls -sh dist/pyoxy/pyoxy-modules/
  fi

  ls -lh dist/pyoxy/
  ./dist/pyoxy/pyoxy -V

  if [ -n "{{libraries}}" ]; then
    baked="baked libraries: {{libraries}}"
  else
    baked="standard library only"
  fi
  if [ -n "{{split}}" ]; then
    mode="split (payload in pyoxy-resources.packed-resources, keep both files together)"
  else
    mode="embedded (single file)"
  fi
  modular=$(ls -d dist/pyoxy/pyoxy-modules/*/ dist/pyoxy/pyoxy-modules/*.packed-resources 2>/dev/null | wc -l)
  echo "ok: dist/pyoxy/pyoxy - $mode, $baked; $modular modular librar(y|ies) beside it"

# Build and run the development pyoxy: it links the system's libpython
# dynamically and inherits its search paths, so it starts instantly (no
# distribution download or static link) and is only for local iteration,
# not for distribution. Interpreter arguments are passed through verbatim:
# `just pyoxy-dev -c 'print(sys.version)'`, `just pyoxy-dev script.py`,
# `just pyoxy-dev -m mymodule`.
pyoxy-dev *args:
  #!/usr/bin/env bash
  set -eo pipefail

  exec cargo run --quiet -p pyoxy -- "$@"

# Produce one `*.packed-resources` file per Python package into
# dist/pyoxy-modules/, ready to copy next to a pyoxy executable under a
# `pyoxy-modules/` directory.
#
# Copying a file in makes the library importable to pyoxy; deleting the file
# removes it. No rebuild of anything is needed either way. Example:
# `just pyoxy-package 3.11 requests pyyaml lxml`.
#
# The first argument is the Python version; it defaults to the version
# `just pyoxy` builds, so runner and library files agree. A file carries a
# `.packed-resources.json` sidecar recording that version, and a runner built
# against a different Python skips it with a warning instead of failing on
# incompatible bytecode. The host's default target triple is used; pass
# `--target-triple` when invoking pyoxidizer directly for overrides.
pyoxy-package python_version="3.11" *req:
  #!/usr/bin/env bash
  set -eo pipefail

  # `*req` is variadic and must come last, so the version is the first
  # positional argument; catch the easy mistake of omitting it.
  case "{{python_version}}" in
    [0-9]*.[0-9]*) ;;
    *) echo "first argument must be the Python version: just pyoxy-package 3.11 requests (got '{{python_version}}')" >&2; exit 1 ;;
  esac

  case "{{req}}" in
    *--unpack*)
      echo "--unpack is a mode, not a requirement: use the unpacked recipe instead" >&2
      echo "  just pyoxy-package-dir {{python_version}} <requirements>" >&2
      exit 1
      ;;
  esac

  cargo run --release --quiet --bin pyoxidizer -- package \
    --python-version {{python_version}} --dest dist/pyoxy-modules {{req}}

# Produce one directory per Python package in dist/pyoxy-modules/, containing
# the package's real files: modules, extension modules (`.so`/`.dll`),
# package data and `.dist-info` metadata.
#
# This is the mode for packages that need native code or a real `__file__`.
# A packed resources file is loaded from memory and cannot carry extension
# modules on Linux, so `numpy`, `pandas`, `psutil` or `pyocd` only work
# unpacked. Each directory is a `sys.path` entry next to the runner; removing
# the directory removes the library, with no rebuild.
#
# The first argument is the Python version, like `pyoxy-package`; the wheel
# ABI must match the runner (cp311 wheels for a 3.11 runner).
pyoxy-package-dir python_version="3.11" *req:
  #!/usr/bin/env bash
  set -eo pipefail

  case "{{python_version}}" in
    [0-9]*.[0-9]*) ;;
    *) echo "first argument must be the Python version: just pyoxy-package-dir 3.11 numpy (got '{{python_version}}')" >&2; exit 1 ;;
  esac

  cargo run --release --quiet --bin pyoxidizer -- package --unpack \
    --python-version {{python_version}} --dest dist/pyoxy-modules {{req}}

# Run the test suites of the pyoxidizer build tool and the pyoxy runner.
test:
  cargo test --quiet -p pyoxidizer -p pyoxy

# Name of the repository section in ~/.pypirc that PyOxidizer wheels are
# uploaded to. This fork is not published on pypi.org.
pypi_repository := env_var_or_default("PYOXIDIZER_PYPI_REPOSITORY", "collmot")

actions-install-sccache-linux:
  python3 scripts/secure_download.py \
    https://github.com/mozilla/sccache/releases/download/v0.3.3/sccache-v0.3.3-x86_64-unknown-linux-musl.tar.gz \
    427bd2151a1b01cd9b094d842e22c445b30f3c645f171a9a62ea55270f06bf23 \
    sccache.tar.gz
  tar -xvzf sccache.tar.gz
  mv sccache-v0.3.3-x86_64-unknown-linux-musl/sccache /home/runner/.cargo/bin/sccache
  rm -rf sccache*
  chmod +x /home/runner/.cargo/bin/sccache

actions-install-sccache-macos:
  python3 scripts/secure_download.py \
    https://github.com/mozilla/sccache/releases/download/v0.3.3/sccache-v0.3.3-x86_64-apple-darwin.tar.gz \
    e68aa0e2716e9cceff7912e09d8028df34cd63d8d60cae832a2d5f5c94da1828 \
    sccache.tar.gz
  tar -xvzf sccache.tar.gz
  mv sccache-v0.3.3-x86_64-apple-darwin/sccache /Users/runner/.cargo/bin/sccache
  rm -rf sccache*
  chmod +x /Users/runner/.cargo/bin/sccache

actions-install-sccache-windows:
  python3 scripts/secure_download.py \
    https://github.com/mozilla/sccache/releases/download/v0.3.3/sccache-v0.3.3-x86_64-pc-windows-msvc.tar.gz \
    b37b55b6e45737562450a98cd4e5c91e58540febdbbda5c575acf80314a2b3ea \
    sccache.tar.gz
  tar -xvzf sccache.tar.gz
  mv sccache-v0.3.3-x86_64-pc-windows-msvc/sccache.exe C:/Users/runneradmin/.cargo/bin/sccache.exe

actions-bootstrap-rust-linux: actions-install-sccache-linux
  sudo apt install -y --no-install-recommends libpcsclite-dev musl-tools

actions-bootstrap-rust-macos: actions-install-sccache-macos

actions-bootstrap-rust-windows: actions-install-sccache-windows

actions-macos-universal exe:
  #!/usr/bin/env bash
  set -eo pipefail

  mkdir -p uploads
  {{lipo_command}} {{exe}}-x86-64/{{exe}} {{exe}}-aarch64/{{exe}} -create -output uploads/{{exe}}
  chmod +x uploads/{{exe}}
  {{lipo_command}} uploads/{{exe}} -info

  # There might be a COPYING file with licensing info. If so, preserve it.
  if [ -e "{{exe}}-aarch64/COPYING" ]; then
    cp -v {{exe}}-aarch64/COPYING uploads/
  fi

actions-build-pyoxy-linux target_triple python_version:
  mkdir -p pyoxy/build target
  chmod 777 pyoxy/build target
  docker run \
    --rm \
    -v $(pwd):/pyoxidizer \
    -v /usr/local/bin/pyoxidizer:/usr/bin/pyoxidizer \
    pyoxidizer:build \
    /pyoxidizer/ci/build-pyoxy-linux.sh {{target_triple}} {{python_version}} build

  mkdir upload
  cp pyoxy/build/{{target_triple}}/release/pyoxy upload/
  cp pyoxy/build/{{target_triple}}/release/resources/COPYING.txt upload/COPYING

# Build PyOxy binary on Linux (local dev mode).
pyoxy-build-linux target_triple python_version:
  #!/usr/bin/env bash
  set -eo pipefail

  (cd ci && docker build -f linux-portable-binary.Dockerfile -t linux-portable-binary:latest .)
  cargo build --bin pyoxidizer --target x86_64-unknown-linux-musl

  rm -rf target/docker
  mkdir -p target/docker

  docker run \
    --rm \
    -it \
    -v $(pwd):/pyoxidizer \
    -v $(pwd)/target/x86_64-unknown-linux-musl/debug/pyoxidizer:/usr/bin/pyoxidizer \
    linux-portable-binary:latest \
    /pyoxidizer/ci/build-pyoxy-linux.sh {{target_triple}} {{python_version}} ../target/docker

pyoxy-build-linux-stage pyoxy_version triple python_version:
  #!/usr/bin/env bash
  set -eo pipefail

  just pyoxy-build-linux {{triple}} {{python_version}}

  DEST_DIR=dist/pyoxy-{{pyoxy_version}}-{{triple}}-python{{python_version}}
  mkdir -p ${DEST_DIR}

  cp target/docker/{{triple}}/release/pyoxy ${DEST_DIR}/
  cp target/docker/{{triple}}/release/resources/COPYING.txt ${DEST_DIR}/COPYING

# Build all PyOxy binaries for Linux.
pyoxy-build-linux-all:
  #!/usr/bin/env bash
  set -eo pipefail

  PYOXY_VERSION=$(cargo metadata --manifest-path pyoxy/Cargo.toml --no-deps | jq --raw-output '.packages[] | select(.name == "pyoxy") | .version')

  just pyoxy-build-linux-stage ${PYOXY_VERSION} x86_64-unknown-linux-gnu 3.10
  just pyoxy-build-linux-stage ${PYOXY_VERSION} x86_64-unknown-linux-gnu 3.11
  just pyoxy-build-linux-stage ${PYOXY_VERSION} x86_64-unknown-linux-gnu 3.12
  just pyoxy-build-linux-stage ${PYOXY_VERSION} x86_64-unknown-linux-gnu 3.13
  just pyoxy-build-linux-stage ${PYOXY_VERSION} x86_64-unknown-linux-gnu 3.14

actions-build-pyoxy-macos triple python_version:
  #!/usr/bin/env bash
  set -euxo pipefail

  export SDKROOT=/Applications/Xcode_15.0.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX14.0.sdk
  export MACOSX_DEPLOYMENT_TARGET={{macosx_deployment_target}}
  pyoxidizer build --release --target-triple {{triple}} --path pyoxy --var PYTHON_VERSION {{python_version}}
  PYO3_CONFIG_FILE=$(pwd)/pyoxy/build/{{triple}}/release/resources/pyo3-build-config-file.txt cargo build --bin pyoxy --target {{triple}} --release

  mkdir upload
  cp target/{{triple}}/release/pyoxy upload/
  cp pyoxy/build/{{triple}}/release/resources/COPYING.txt upload/COPYING
  sccache --stop-server

# Trigger a workflow on a branch.
ci-run workflow branch="ci-test":
  gh workflow run {{workflow}} --ref {{branch}}

# Trigger all workflows on a given branch.
ci-run-all branch="ci-test":
  just ci-run cargo_deny.yml {{branch}}
  just ci-run oxidized_importer.yml {{branch}}
  just ci-run pyoxidizer.yml {{branch}}
  just ci-run pyoxy.yml {{branch}}
  just ci-run sphinx.yml {{branch}}
  just ci-run workspace.yml {{branch}}
  just ci-run workspace-python.yml {{branch}}

_remote-sign-exe ref workflow run_id artifact exe_name rcodesign_branch="main":
  gh workflow run sign-apple-exe.yml \
    --ref {{ref}} \
    -f workflow={{workflow}} \
    -f run_id={{run_id}} \
    -f artifact={{artifact}} \
    -f exe_name={{exe_name}} \
    -f rcodesign_branch={{rcodesign_branch}}

# Trigger remote code signing workflow for pyoxy executable.
remote-sign-pyoxy ref run_id rcodesign_branch="main": (_remote-sign-exe ref "rcodesign.yml" run_id "exe-pyoxy-macos-universal" "pyoxy" rcodesign_branch)

# Obtain built executables from GitHub Actions.
assemble-exe-artifacts exe commit dest:
  #!/usr/bin/env bash
  set -exo pipefail

  RUN_ID=$(gh run list \
    --workflow {{exe}}.yml \
    --json databaseId,headSha | \
    jq --raw-output '.[] | select(.headSha=="{{commit}}") | .databaseId' | head -n 1)

  if [ -z "${RUN_ID}" ]; then
    echo "could not find GitHub Actions run with artifacts"
    exit 1
  fi

  echo "GitHub run ID: ${RUN_ID}"

  gh run download --dir {{dest}} ${RUN_ID}

_codesign-exe in_path:
  rcodesign sign \
    --remote-signer \
    --remote-public-key-pem-file ci/developer-id-application.pem \
    --code-signature-flags runtime \
    {{in_path}}

_codesign in_path out_path:
  rcodesign sign \
    --remote-signer \
    --remote-public-key-pem-file ci/developer-id-application.pem \
    {{in_path}} {{out_path}}

# Notarize and staple a path.
notarize path:
  rcodesign notarize \
    --api-issuer 254e4e96-2b8b-43c1-b385-286bdad51dba \
    --api-key 8RXL6MN9WV \
    --staple \
    {{path}}

_tar_directory source_directory dir_name dest_dir:
  # We need GNU tar here so we use tar on Linux and gtar on macOS
  {{tar_command}} \
    --sort=name \
    --owner=root:0 \
    --group=root:0 \
    --mtime="2022-01-01 00:00:00" \
    -C {{source_directory}} \
    -cvzf {{dest_dir}}/{{dir_name}}.tar.gz \
    {{dir_name}}/

_zip_directory source_directory dir_name dest_dir:
  #!/usr/bin/env bash
  set -exo pipefail

  here=$(pwd)

  cd {{source_directory}}
  zip -r ${here}/{{dest_dir}}/{{dir_name}}.zip {{dir_name}}

_release_universal_binary project tag exe:
  mkdir -p dist/{{project}}-stage/{{project}}-{{tag}}-macos-universal
  {{lipo_command}} \
    -create \
    -output dist/{{project}}-stage/{{project}}-{{tag}}-macos-universal/{{exe}} \
    dist/{{project}}-stage/{{project}}-{{tag}}-aarch64-apple-darwin/{{exe}} \
    dist/{{project}}-stage/{{project}}-{{tag}}-x86_64-apple-darwin/{{exe}}
  cp dist/{{project}}-stage/{{project}}-{{tag}}-aarch64-apple-darwin/COPYING \
    dist/{{project}}-stage/{{project}}-{{tag}}-macos-universal/COPYING

_create_shasums dir:
  #!/usr/bin/env bash
  set -exo pipefail

  (cd {{dir}} && shasum -a 256 *.* > SHA256SUMS)

  for p in {{dir}}/*.*; do
    if [[ "${p}" != *"SHA256SUMS" ]]; then
      shasum -a 256 $p | awk '{print $1}' > ${p}.sha256
    fi
  done

_upload_release name title_name commit tag:
  git tag -f {{name}}/{{tag}} {{commit}}
  git push -f origin refs/tags/{{name}}/{{tag}}:refs/tags/{{name}}/{{tag}}
  gh release create \
    --prerelease \
    --target {{commit}} \
    --title '{{title_name}} {{tag}}' \
    --discussion-category general \
    {{name}}/{{tag}}
  gh release upload --clobber {{name}}/{{tag}} dist/{{name}}/*

_release name title_name:
  #!/usr/bin/env bash
  set -exo pipefail

  COMMIT=$(git rev-parse HEAD)
  TAG=$(cargo metadata \
    --manifest-path {{name}}/Cargo.toml \
    --format-version 1 \
    --no-deps | \
      jq --raw-output '.packages[] | select(.name=="{{name}}") | .version')

  just {{name}}-release-prepare ${COMMIT} ${TAG}
  just {{name}}-release-upload ${COMMIT} ${TAG}

# Prepare PyOxy release artifacts.
pyoxy-release-prepare commit tag:
  #!/usr/bin/env bash
  set -exo pipefail

  rm -rf dist/pyoxy*
  just assemble-exe-artifacts pyoxy {{commit}} dist/pyoxy-artifacts

  for py in 3.10 3.11 3.12 3.13 3.14; do
    for triple in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu macos-universal; do
      release_name=pyoxy-{{tag}}-${triple}-cpython${py}
      source=dist/pyoxy-artifacts/exe-pyoxy-${triple}-${py}
      dest=dist/pyoxy-stage/${release_name}
      mkdir -p ${dest}
      cp -a ${source}/pyoxy ${dest}/pyoxy
      # GitHub Actions zip files don't preserve executable bit.
      chmod +x ${dest}/pyoxy
      cp -a ${source}/COPYING ${dest}/COPYING

      case ${triple} in
        *apple* | macos-universal)
          # just _codesign-exe ${dest}/pyoxy
          ;;
        *)
          ;;
      esac

      mkdir -p dist/pyoxy

      just _tar_directory \
        dist/pyoxy-stage \
        ${release_name} \
        dist/pyoxy
    done
  done

  just _create_shasums dist/pyoxy

# Upload PyOxy release artifacts to a new GitHub release.
pyoxy-release-upload commit tag:
  just _upload_release pyoxy PyOxy {{commit}} {{tag}}

# Perform a PyOxy release end-to-end.
pyoxy-release:
  just _release pyoxy PyOxy

oxidized_importer-release-prepare commit tag:
  #!/usr/bin/env bash
  set -exo pipefail

  rm -rf dist/oxidized_importer*

  just assemble-exe-artifacts oxidized_importer {{commit}} dist/oxidized_importer-artifacts
  mkdir dist/oxidized_importer
  cp dist/oxidized_importer-artifacts/wheels/*.whl dist/oxidized_importer

  just _create_shasums dist/oxidized_importer

oxidized_importer-release-upload commit tag:
  just _upload_release oxidized_importer 'Oxidized Importer Python Extension' {{commit}} {{tag}}
  twine upload dist/oxidized_importer/*.whl

oxidized_importer-release commit tag:
  just oxidized_importer-release-prepare {{commit}} {{tag}}
  just oxidized_importer-release-upload {{commit}} {{tag}}

# Create a .dmg for PyOxidizer
pyoxidizer-create-dmg:
  #!/usr/bin/env bash
  set -exo pipefail

  # Clear out old state.
  rm -rf build dmg_root PyOxidizer.dmg dist/pyoxidizer*

  if [ -d /Volumes/PyOxidizer ]; then
    DEV_NAME=$(hdiutil info | egrep --color=never '^/dev/' | sed 1q | awk '{print $1}')
    hdiutil detach "${DEV_NAME}"
  fi

  if [[ $(uname -m) == 'arm64' ]]; then
    PYOXIDIZER=target/aarch64-apple-darwin/release/pyoxidizer
  else
    PYOXIDIZER=target/x86_64-apple-darwin/release/pyoxidizer
  fi

  $PYOXIDIZER build --release macos_app_bundle
  just _codesign build/*/release/macos_app_bundle/PyOxidizer.app dist/pyoxidizer-stage/PyOxidizer.app

  hdiutil create \
    -srcfolder dist/pyoxidizer-stage \
    -volname PyOxidizer \
    -fs HFS+ \
    -fsargs "-c c=64,a=16,e=16" \
    -format UDRW \
    PyOxidizer

  # Mount it.
  DEV_NAME=$(hdiutil attach -readwrite -noverify -noautoopen PyOxidizer.dmg | egrep --color=never '^/dev/' | sed 1q | awk '{print $1}')

  # Create a symlink to /Applications for drag and drop.
  ln -s /Applications /Volumes/PyOxidizer/Applications

  # Run AppleScript to create the .DS_Store.
  /usr/bin/osascript scripts/dmg.applescript PyOxidizer

  # --openfolder not supported on ARM.
  if [[ $(uname -m) == "arm64" ]]; then
    bless --folder /Volumes/PyOxidizer
  else
    bless --folder /Volumes/PyOxidizer --openfolder /Volumes/PyOxidizer
  fi

  # Unmount.
  hdiutil detach "${DEV_NAME}"

  # Compress.
  hdiutil convert PyOxidizer.dmg -format UDZO -imagekey zlib-level=9 -ov -o PyOxidizer.dmg
  just _codesign PyOxidizer.dmg PyOxidizer.dmg
  just notarize PyOxidizer.dmg

# Prepare PyOxidizer release artifacts.
pyoxidizer-release-prepare commit tag:
  #!/usr/bin/env bash
  set -exo pipefail

  rm -rf dist/pyoxidizer*

  just assemble-exe-artifacts pyoxidizer {{commit}} dist/pyoxidizer-artifacts

  mkdir dist/pyoxidizer dist/pyoxidizer-stage

  # Windows installers are easy.
  cp -av dist/pyoxidizer-artifacts/pyoxidizer-windows_installers/*.{exe,msi} dist/pyoxidizer/

  # Assemble plain executable releases.
  for triple in aarch64-apple-darwin aarch64-unknown-linux-musl i686-pc-windows-msvc x86_64-apple-darwin x86_64-pc-windows-msvc x86_64-unknown-linux-musl; do
    release_name=pyoxidizer-{{tag}}-${triple}
    source=dist/pyoxidizer-artifacts/exe-pyoxidizer-${triple}
    dest=dist/pyoxidizer-stage/${release_name}

    exe=pyoxidizer
    sign_command=
    archive_action=_tar_directory

    case ${triple} in
      *apple*)
        # sign_command="just _codesign-exe ${dest}/${exe}"
        ;;
      *windows*)
        exe=pyoxidizer.exe
        archive_action=_zip_directory
        ;;
      *)
        ;;
    esac

    mkdir -p ${dest}
    cp -a ${source}/${exe} ${dest}/${exe}
    chmod +x ${dest}/${exe}

    if [ -n "${sign_command}" ]; then
      ${sign_command}
    fi

    cargo run --bin pyoxidizer -- rust-project-licensing \
      --system-rust \
      --target-triple ${triple} \
      --all-features \
      --unified-license \
      pyoxidizer > ${dest}/COPYING

    just ${archive_action} dist/pyoxidizer-stage ${release_name} dist/pyoxidizer

    # Wheels are built using the (signed) executables in the archives.
    mkdir -p target/${triple}/release
    cp ${dest}/${exe} target/${triple}/release/
    cargo run --bin pyoxidizer -- build --release wheel_${triple}

    cp build/*/release/wheel_${triple}/*.whl dist/pyoxidizer/
  done

  # Create universal binary from signed single arch Mach-O binaries.
  just _release_universal_binary pyoxidizer {{tag}} pyoxidizer
  just _tar_directory dist/pyoxidizer-stage pyoxidizer-{{tag}}-macos-universal dist/pyoxidizer

  # The DMG is created using the signed binaries.
  # for triple in aarch64-apple-darwin x86_64-apple-darwin; do
  #   ssh macmini mkdir -p /Users/gps/src/PyOxidizer/target/${triple}/release
  #   scp dist/pyoxidizer-stage/pyoxidizer-{{tag}}-${triple}/pyoxidizer macmini:~/src/PyOxidizer/target/${triple}/release/
  # done
  # ssh macmini just -d /Users/gps/src/PyOxidizer -f /Users/gps/src/PyOxidizer/Justfile pyoxidizer-create-dmg
  # scp macmini:~/src/PyOxidizer/PyOxidizer.dmg dist/pyoxidizer/PyOxidizer-{{tag}}.dmg

  just _create_shasums dist/pyoxidizer

# Upload PyOxidizer release wheels to the private package index.
#
# The release commit and tag are created by `cargo release` (see the
# `release-rust` recipe), and no GitHub release is created for this fork, so
# the only thing left to upload are the wheels built by
# `pyoxidizer-release-prepare`. The `commit` and `tag` arguments are accepted
# for compatibility with the `_release` helper.
pyoxidizer-release-upload commit tag:
  twine upload --repository {{pypi_repository}} dist/pyoxidizer/*.whl

# Perform release automation for PyOxidizer.
pyoxidizer-release:
  just _release pyoxidizer 'PyOxidizer'

# Updates Python distributions embedded in PyOxidizer.
pyoxidizer-update-python-distributions: _python_scripts_venv
  venv/bin/python3 \
    scripts/fetch-python-distributions.py \
    --tag 20261003 > pyoxidizer/src/default_python_distributions.rs
  rustfmt pyoxidizer/src/default_python_distributions.rs

# Perform Rust crate releases.
release-rust:
  cargo release release --exclude pyoxidizer --execute
  cargo release release -p pyoxidizer --execute

# Create virtual environment and install dependencies for Python scripts in it.
_python_scripts_venv:
  #!/usr/bin/env bash
  if [ ! -d venv ]; then
    python3 -m venv venv
    venv/bin/pip install -r scripts/requirements.txt
  fi
