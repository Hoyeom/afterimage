# Signs a draft release built by .github/workflows/release.yml and publishes it.
# Needs the signing key on this PC; see examples/sign.rs.
#
#   scripts\publish.ps1 0.4.0
param([Parameter(Mandatory = $true)][string]$Version)

$ErrorActionPreference = 'Stop'
$repo = 'Hoyeom/afterimage'
$tag = "v$Version"
$dir = Join-Path ([IO.Path]::GetTempPath()) "afterimage-publish-$PID"
New-Item -ItemType Directory -Path $dir | Out-Null

try {
    gh release download $tag --repo $repo --pattern afterimage.exe --dir $dir
    if ($LASTEXITCODE -ne 0) { throw "Couldn't download afterimage.exe from $tag" }

    $line = cargo run --quiet --example sign -- manifest $Version (Join-Path $dir 'afterimage.exe')
    if ($LASTEXITCODE -ne 0) { throw 'Signing failed' }

    # No BOM: the app reads this file as plain UTF-8.
    $manifest = Join-Path $dir 'latest.txt'
    [IO.File]::WriteAllText($manifest, "$line`n")

    gh release upload $tag $manifest --repo $repo --clobber
    if ($LASTEXITCODE -ne 0) { throw "Couldn't upload latest.txt" }

    gh release edit $tag --repo $repo --draft=false --latest
    if ($LASTEXITCODE -ne 0) { throw "Couldn't publish $tag" }

    Write-Output "Published $tag"
} finally {
    Remove-Item -Recurse -Force $dir
}
