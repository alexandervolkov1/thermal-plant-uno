# Run outside the repository so the AVR-only .cargo/config.toml is not inherited.
$repoPath = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$toolchain = 'nightly-2025-04-27'
Push-Location ([IO.Path]::GetTempPath())
try {
    cargo "+$toolchain" test --locked --manifest-path "$PSScriptRoot/Cargo.toml" --target x86_64-pc-windows-msvc --target-dir "$repoPath/target/host-tests"
    if ($LASTEXITCODE -ne 0) { throw 'Host protocol tests failed.' }
} finally {
    Pop-Location
}
