# largo installer (Windows) — the LOGOS build tool.
#
#   powershell -ExecutionPolicy Bypass -c "irm https://logicaffeine.com/install.ps1 | iex"
#
# Pinned version / full flavor:
#   & ([scriptblock]::Create((irm https://logicaffeine.com/install.ps1))) -Full -Version v0.10.0
#
# Installs largo.exe to %LOCALAPPDATA%\Programs\largo and adds that folder
# to your *user* PATH. Downloads are verified against the release's
# SHA256SUMS.
#
# Note on SmartScreen: the binaries are not yet Authenticode-signed.
# Running largo from a terminal does not trip the SmartScreen UI, and this
# script unblocks the extracted file (Mark-of-the-Web); code signing is a
# planned follow-up.

param(
    [switch]$Full,
    [string]$Version = "",
    [string]$To = ""
)

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 defaults to TLS 1.0 — force 1.2 for GitHub.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$BaseUrl = if ($env:LARGO_BASE_URL) { $env:LARGO_BASE_URL } else { "https://github.com/Brahmastra-Labs/logicaffeine" }

$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -eq 'ARM64') {
    Write-Host "note: running the x64 build under Windows ARM64 emulation (no native arm64 build yet)."
} elseif ($arch -ne 'AMD64') {
    throw "unsupported architecture '$arch' — try: cargo install logicaffeine-cli"
}

$flavor = if ($Full) { "-full" } else { "" }
$asset = "largo$flavor-win32-x64.zip"

# Resolve the release tag without the GitHub API (rate-limit-proof).
if (-not $Version) {
    try {
        $resp = Invoke-WebRequest -Uri "$BaseUrl/releases/latest" -MaximumRedirection 0 -UseBasicParsing -ErrorAction SilentlyContinue
        $location = $resp.Headers.Location
    } catch {
        $location = $_.Exception.Response.Headers.Location
    }
    # PowerShell 7 surfaces Headers.Location as string[]; 5.1 as a scalar.
    if ($location -is [System.Array]) { $location = $location[0] }
    if (-not $location) { throw "cannot resolve the latest release from $BaseUrl" }
    $Version = ($location.ToString().TrimEnd('/') -split '/')[-1]
}
if ($Version -notmatch '^v') { $Version = "v$Version" }

$download = "$BaseUrl/releases/download/$Version"
$tmp = Join-Path $env:TEMP "largo-install-$PID"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

try {
    Write-Host "downloading largo $Version (win32-x64$(if ($Full) { ', full' })) ..."
    $zipPath = Join-Path $tmp $asset
    Invoke-WebRequest -Uri "$download/$asset" -OutFile $zipPath -UseBasicParsing

    $sumsPath = Join-Path $tmp "SHA256SUMS"
    Invoke-WebRequest -Uri "$download/SHA256SUMS" -OutFile $sumsPath -UseBasicParsing

    $expected = (Get-Content $sumsPath | Where-Object { $_ -match [regex]::Escape($asset) } | ForEach-Object { ($_ -split '\s+')[0] })
    if (-not $expected) { throw "no checksum entry for $asset — refusing to install" }
    $actual = (Get-FileHash -Algorithm SHA256 $zipPath).Hash
    if ($actual -ne $expected -and $actual.ToLower() -ne $expected.ToLower()) {
        throw "checksum mismatch for $asset — the download is corrupt or tampered with; nothing was installed"
    }

    $extract = Join-Path $tmp "extract"
    Expand-Archive -Path $zipPath -DestinationPath $extract -Force
    $exe = Join-Path $extract "largo.exe"
    if (-not (Test-Path $exe)) { throw "archive did not contain largo.exe" }
    Unblock-File $exe

    $installDir = if ($To) { $To } elseif ($env:LARGO_INSTALL_DIR) { $env:LARGO_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Programs\largo" }
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Move-Item -Force $exe (Join-Path $installDir "largo.exe")

    $installed = & (Join-Path $installDir "largo.exe") --version
    Write-Host "installed largo $installed -> $installDir\largo.exe"

    # User PATH (never the machine PATH).
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not ($userPath -split ';' | Where-Object { $_ -eq $installDir })) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
        $env:Path = "$env:Path;$installDir"
        Write-Host "added $installDir to your user PATH (new terminals pick it up automatically)."
    }

    Write-Host ""
    Write-Host "get started:  largo new hello; cd hello; largo run"
    Write-Host "uninstall:    remove $installDir and its PATH entry"
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
