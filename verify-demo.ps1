param(
    [ValidateRange(0, 65535)]
    [int]$Port = 0
)

$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot 'dist\pg-debugger.exe'
if (-not (Test-Path -LiteralPath $exe)) {
    throw "Не найден $exe. Сначала соберите или распакуйте демонстрационный exe."
}

if ($Port -eq 0) {
    $probe = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $probe.Start()
    $Port = ([System.Net.IPEndPoint]$probe.LocalEndpoint).Port
    $probe.Stop()
}

# Отдельная папка не затрагивает кластер, который использует обычный запуск программы.
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('pg-debugger-check-' + [guid]::NewGuid().ToString('N'))
$localAppData = Join-Path $testRoot 'local'
New-Item -ItemType Directory -Force -Path $localAppData | Out-Null
$inputFile = Join-Path $testRoot 'input.txt'
$stdoutFile = Join-Path $testRoot 'stdout.txt'
$stderrFile = Join-Path $testRoot 'stderr.txt'
$lines = @($Port.ToString(), '1', '2', 'src', 'b 6', 'bl', 'c', 'd 6', 'bl', 'c', '4', 'n', 'n', '5', 'public.debug_smoke(integer)', '6', 'public.debug_smoke(integer)', '6', '1', 'SELECT public.debug_smoke(3);', 'c', '6', 'public.debug_smoke(integer)', '3', '1', 'SELECT public.debug_smoke(5);', 'c', '6', 'public.debug_smoke(integer)', '4', '1', 'SELECT public.debug_smoke(5);', 'c', '0')
[System.IO.File]::WriteAllText($inputFile, ($lines -join "`r`n") + "`r`n", [System.Text.Encoding]::ASCII)

$previousLocalAppData = $env:LOCALAPPDATA
try {
    $env:LOCALAPPDATA = $localAppData
    $process = Start-Process -FilePath $exe -WorkingDirectory $PSScriptRoot `
        -RedirectStandardInput $inputFile -RedirectStandardOutput $stdoutFile `
        -RedirectStandardError $stderrFile -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(120000)) {
        $process.Kill()
        throw "Проверка заняла больше двух минут. Журнал и вывод сохранены в $testRoot"
    }
}
finally {
    $env:LOCALAPPDATA = $previousLocalAppData
}

$stdout = [System.IO.File]::ReadAllText($stdoutFile, [System.Text.Encoding]::UTF8)
$stderr = [System.IO.File]::ReadAllText($stderrFile, [System.Text.Encoding]::UTF8)
$process.Refresh()
if (-not $process.HasExited) {
    throw "Отладчик не завершился. Файлы проверки: $testRoot`n$stdout`n$stderr"
}

$expected = @(
    'Global breakpoint set for public.debug_smoke(integer)',
    'After step over: debug_smoke(integer):6',
    'running_total (declared at line 3): 4',
    'Breakpoint set at source line 6.',
    'Breakpoint removed from source line 6.',
    'Target SQL failed (SQLSTATE P0001)',
    'Синтаксис тела принят валидатором PL/pgSQL. Тело не выполнялось.',
    'Global breakpoint set for public.debug_smoke(integer) at source line 6',
    'Строка 3 находится до первого оператора или на нём; ставлю точку на входе.',
    'Строка 4 находится до первого оператора или на нём; ставлю точку на входе.',
    'Stopped at debug_smoke(integer):5',
    'Готово.'
)
foreach ($part in $expected) {
    if (-not $stdout.Contains($part)) {
        throw "Не найден ожидаемый фрагмент: $part. Файлы проверки: $testRoot`n$stdout`n$stderr"
    }
}

if (([regex]::Matches($stdout, 'Stopped at debug_smoke\(integer\):5')).Count -lt 3) {
    throw "Ожидались остановки на фактической строке 5, включая запросы строк 3 и 4. Файлы проверки: $testRoot`n$stdout`n$stderr"
}

if (([regex]::Matches($stdout, '(?m)^\s*8\s*$')).Count -lt 2) {
    throw "Ожидались два результата 8 от psql. Файлы проверки: $testRoot`n$stdout`n$stderr"
}

Write-Host 'Проверка пройдена: шаг, точка на строке и её удаление, проверка тела, ошибка P0001, результат 8 и остановка сервера.'

# Удаляем только созданную этим запуском временную папку после успешной проверки.
$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\')
$resolvedTestRoot = [System.IO.Path]::GetFullPath($testRoot)
if (-not $resolvedTestRoot.StartsWith($tempRoot + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Временная папка оказалась вне TEMP: $resolvedTestRoot"
}
Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
