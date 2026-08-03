$ErrorActionPreference = "Stop"

$repo = "hugues31/telos-sdd"
$version = $env:TELOS_VERSION
$installDir = $env:TELOS_INSTALL_DIR
if (-not $installDir) {
    $installDir = Join-Path $HOME ".local\bin"
}
if (-not $version) {
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
    $version = $release.tag_name
}

$arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    "X64" { "amd64" }
    "Arm64" { "arm64" }
    default { throw "Unsupported architecture: $_" }
}
$plainVersion = $version.TrimStart("v")
$archive = "telos_${plainVersion}_windows_${arch}.zip"
$baseUrl = "https://github.com/$repo/releases/download/$version"
$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid().ToString())

try {
    New-Item -ItemType Directory -Path $tmpDir | Out-Null
    Invoke-WebRequest "$baseUrl/$archive" -OutFile (Join-Path $tmpDir $archive)
    Invoke-WebRequest "$baseUrl/checksums.txt" -OutFile (Join-Path $tmpDir "checksums.txt")
    $checksumLine = Get-Content (Join-Path $tmpDir "checksums.txt") | Where-Object { $_ -match "\s$([regex]::Escape($archive))$" }
    if (-not $checksumLine) { throw "Release checksum does not contain $archive." }
    $expected = ($checksumLine -split "\s+")[0].ToLowerInvariant()
    $actual = (Get-FileHash (Join-Path $tmpDir $archive) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "Checksum verification failed for $archive." }
    Expand-Archive (Join-Path $tmpDir $archive) -DestinationPath $tmpDir
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Copy-Item (Join-Path $tmpDir "telos.exe") (Join-Path $installDir "telos.exe") -Force
    Write-Host "Installed Telos $version to $(Join-Path $installDir 'telos.exe')"
} finally {
    if (Test-Path $tmpDir) { Remove-Item -Recurse -Force $tmpDir }
}

