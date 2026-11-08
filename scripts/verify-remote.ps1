param(
    [string]$RemoteHost = 'test-env',
    [string]$RemoteDirectory = '/tmp/weread-implementation',
    [string]$RemoteTargetDirectory = '/root/weread-build/target',
    [string]$DesignReferenceDirectory = '',
    [switch]$SkipUi,
    [switch]$Design,
    [switch]$Kindle,
    [ValidateRange(1, 2147483647)]
    [int]$CargoBuildJobs = 12
)

$ErrorActionPreference = 'Stop'
$WereadRepository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
foreach ($WereadRemotePath in @($RemoteDirectory, $RemoteTargetDirectory)) {
    if ($WereadRemotePath -notmatch '^/[A-Za-z0-9_./-]+$' -or $WereadRemotePath.Contains('..')) {
        throw 'Remote paths must be simple, absolute workspace paths without parent traversal.'
    }
}
if ($DesignReferenceDirectory -ne '' -and ($DesignReferenceDirectory -notmatch '^/[A-Za-z0-9_./-]+$' -or $DesignReferenceDirectory.Contains('..'))) {
    throw 'The optional design reference must be a simple absolute path on the remote worker.'
}
if ($RemoteHost -notmatch '^[A-Za-z0-9_][A-Za-z0-9_.@-]*$') {
    throw 'The remote host must be an SSH alias or a simple hostname.'
}

& ssh $RemoteHost "mkdir -p '$RemoteDirectory' '$RemoteTargetDirectory'"
if ($LASTEXITCODE -ne 0) { throw 'The designated remote verification environment is unavailable.' }

$WereadSources = @('Cargo.toml', 'Cargo.lock', 'build.rs', '.cargo', 'src', 'ui', 'assets', 'vendor', 'scripts', 'packaging', 'tools')
$WereadTransferPaths = foreach ($WereadSource in $WereadSources) {
    $WereadPath = Join-Path $WereadRepository $WereadSource
    if (Test-Path -LiteralPath $WereadPath) { $WereadPath }
}
& scp -r @WereadTransferPaths "${RemoteHost}:${RemoteDirectory}/"
if ($LASTEXITCODE -ne 0) { throw 'Source transfer failed.' }

$WereadRemoteCommands = @(
    'set -eu',
    'if [ -f /root/.cargo/env ]; then . /root/.cargo/env; fi',
    "export CARGO_TARGET_DIR='$RemoteTargetDirectory' CARGO_BUILD_JOBS=$CargoBuildJobs",
    "cd '$RemoteDirectory'",
    'cargo fmt --all -- --check',
    'python3 scripts/check-translations.py',
    'cargo test --locked --all-targets',
    'python3 packaging/tests/launcher_test.py',
    'cargo build --locked'
)
if (-not $SkipUi) {
    $WereadRemoteCommands += 'python3 scripts/verify-ui.py --binary "$CARGO_TARGET_DIR/debug/weread" --outputdir artifacts/screenshots'
}
if ($Design) {
    $WereadRemoteCommands += 'cargo build --manifest-path tools/offline-ui-fixture/Cargo.toml'
    $WereadDesignCommand = 'python3 scripts/verify-design-fidelity.py --binary "$CARGO_TARGET_DIR/debug/weread" --state-harness "$CARGO_TARGET_DIR/debug/weread-offline-ui-fixture" --output-dir artifacts/design-verification'
    if ($DesignReferenceDirectory -ne '') { $WereadDesignCommand += " --reference-dir '$DesignReferenceDirectory'" }
    $WereadRemoteCommands += $WereadDesignCommand
}
if ($Kindle) {
    $WereadRemoteCommands += 'sh scripts/build-kindle.sh'
    $WereadRemoteCommands += 'python3 scripts/package-development.py'
}
& ssh $RemoteHost ($WereadRemoteCommands -join "`n")
if ($LASTEXITCODE -ne 0) { throw 'Remote verification failed; inspect the remote diagnostics and artifacts.' }

Write-Output "Remote verification completed on ${RemoteHost}. Artifacts: ${RemoteDirectory}/artifacts"
