param(
    [string]$Version = "latest",
    [string]$InstallDir = "$HOME\.local\bin"
)

$ErrorActionPreference = "Stop"

$repo = "cccuong-jason/openagents-kit"
$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()

switch ($arch) {
    "x64" { $target = "x86_64-pc-windows-msvc" }
    default { throw "Unsupported Windows architecture: $arch" }
}

$asset = "openagents-kit-$target.exe"
$releaseBase = if ($Version -eq "latest") {
    "https://github.com/$repo/releases/latest/download"
} else {
    "https://github.com/$repo/releases/download/$Version"
}

$tempFile = Join-Path ([System.IO.Path]::GetTempPath()) $asset
$destinationDir = [System.IO.Path]::GetFullPath($InstallDir)
$destination = Join-Path $destinationDir "openagents-kit.exe"

New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
Invoke-WebRequest -Uri "$releaseBase/$asset" -OutFile $tempFile
Move-Item -Force $tempFile $destination

Write-Host "Installed openagents-kit to $destination"
Write-Host "Run `openagents-kit setup` to scan local Codex, Claude, and Gemini configs."
