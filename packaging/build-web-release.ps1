$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
$releaseTemp = $null
try {
    if (-not ($env:OS -eq 'Windows_NT')) { throw 'Этот скрипт предназначен для сборки Windows x64.' }
    $rustInfo = rustc -vV
    if ($LASTEXITCODE -ne 0 -or -not ($rustInfo -match 'host: x86_64-pc-windows-msvc')) { throw 'Нужен Rust для x86_64-pc-windows-msvc.' }
    $manifestText = [IO.File]::ReadAllText((Join-Path $projectRoot 'web/server/Cargo.toml'))
    $version = [regex]::Match($manifestText, '(?m)^version = "([^"]+)"').Groups[1].Value
    if (-not $version) { throw 'Не удалось определить версию веб-приложения.' }
    python packaging/check-editor.py
    if ($LASTEXITCODE -ne 0) { throw 'Пакет редактора не соответствует исходникам.' }
    cargo build --release --locked --manifest-path web/server/Cargo.toml
    if ($LASTEXITCODE -ne 0) { throw 'Сборка завершилась с ошибкой.' }
    $buildTarget = $env:CARGO_TARGET_DIR
    if (-not $buildTarget) { $buildTarget = Join-Path $projectRoot 'web/server/target' }
    $binary = Join-Path $buildTarget 'release/pg-debugger-web.exe'
    $name = "pg-debugger-web-$version-windows-x64"
    $dist = Join-Path $projectRoot 'dist/web'
    New-Item -ItemType Directory -Path $dist -Force | Out-Null
    $releaseTemp = Join-Path ([IO.Path]::GetTempPath()) ('pg-debugger-web-release-' + [guid]::NewGuid().ToString('N'))
    $payload = Join-Path $releaseTemp $name
    New-Item -ItemType Directory -Path $payload -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $payload 'pg-debugger-web.exe')
    Copy-Item -LiteralPath 'README.md' -Destination (Join-Path $payload 'README.md')
    Copy-Item -LiteralPath 'web/prototype/vendor/THIRD_PARTY_LICENSES.txt' -Destination $payload
    $archive = Join-Path $dist ($name + '.zip')
    Compress-Archive -LiteralPath $payload -DestinationPath $archive -Force
    $digest = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText($archive + '.sha256', $digest + '  ' + [IO.Path]::GetFileName($archive) + "`n", [Text.UTF8Encoding]::new($false))

    Write-Output $archive
} finally {
    if ($releaseTemp -and (Test-Path -LiteralPath $releaseTemp)) {
        $resolvedTemp = [IO.Path]::GetFullPath($releaseTemp)
        $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
        if (-not $resolvedTemp.StartsWith($tempRoot,[StringComparison]::OrdinalIgnoreCase) -or -not ([IO.Path]::GetFileName($resolvedTemp).StartsWith('pg-debugger-web-release-'))) { throw 'Неожиданный путь временной сборки.' }
        Remove-Item -LiteralPath $resolvedTemp -Recurse -Force
    }
    Pop-Location
}
