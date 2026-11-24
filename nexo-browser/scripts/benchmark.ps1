<#
.SYNOPSIS
  Воспроизводимый замер памяти, CPU и скорости запуска браузера (Nexo / Chrome / Edge / Brave) на Windows.

.DESCRIPTION
  Для каждого сценария и каждого прогона:
    1. создаётся пустой временный профиль;
    2. браузер запускается с нужными адресами (отдельный профиль => отдельное дерево процессов);
    3. измеряется время до появления главного окна;
    4. после паузы -SettleSeconds измеряется ВСЁ дерево процессов браузера:
       число процессов, Working Set (включает общие страницы), Private Bytes, загрузка CPU за окно;
    5. браузер закрывается, профиль удаляется.
  Дерево процессов определяется по каталогу профиля в командной строке процесса (так находятся и дочерние
  процессы msedgewebview2 у Nexo), поэтому метрика одинакова для всех браузеров.

  Сценарии: blank, one, five, ten, idle10 (десять вкладок и простой -IdleSeconds до замера).
  YouTube-сценарий здесь не автоматизирован (нужна реальная воспроизводимая сессия со звуком) — см. docs\BENCHMARK.md.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\benchmark.ps1 -Browser nexo -Exe .\target\release\nexo.exe
  powershell -ExecutionPolicy Bypass -File scripts\benchmark.ps1 -Browser chrome
  powershell -ExecutionPolicy Bypass -File scripts\benchmark.ps1 -Browser nexo -Exe .\target\release\nexo.exe -Mode memory
#>
param(
  [ValidateSet('nexo', 'chrome', 'edge', 'brave')][string]$Browser = 'nexo',
  [string]$Exe = '',
  [ValidateSet('', 'memory', 'balanced', 'max')][string]$Mode = '',
  [string[]]$Scenarios = @('blank', 'one', 'five', 'ten', 'idle10'),
  [int]$Runs = 3,
  [int]$SettleSeconds = 30,
  [int]$IdleSeconds = 360,
  [int]$CpuWindowSeconds = 20,
  [string]$OutDir = 'bench-out'
)

$ErrorActionPreference = 'Stop'
$Site = 'https://www.wikipedia.org/'
$Cores = [Environment]::ProcessorCount

function Find-Exe([string]$name) {
  if ($Exe) { return (Resolve-Path $Exe).Path }
  $candidates = switch ($name) {
    'chrome' { @("$env:ProgramFiles\Google\Chrome\Application\chrome.exe", "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe", "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe") }
    'edge'   { @("${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe") }
    'brave'  { @("$env:ProgramFiles\BraveSoftware\Brave-Browser\Application\brave.exe", "${env:ProgramFiles(x86)}\BraveSoftware\Brave-Browser\Application\brave.exe", "$env:LOCALAPPDATA\BraveSoftware\Brave-Browser\Application\brave.exe") }
    default  { @('.\target\release\nexo.exe') }
  }
  foreach ($c in $candidates) { if ($c -and (Test-Path $c)) { return (Resolve-Path $c).Path } }
  throw "Не найден $name. Укажите путь параметром -Exe."
}

function Urls-For([string]$scenario) {
  switch ($scenario) {
    'blank'  { if ($Browser -eq 'nexo') { return @() } else { return @('about:blank') } }
    'one'    { return @('https://www.google.com/') }
    'five'   { return @(1..5 | ForEach-Object { $Site }) }
    'ten'    { return @(1..10 | ForEach-Object { $Site }) }
    'idle10' { return @(1..10 | ForEach-Object { $Site }) }
    default  { throw "Неизвестный сценарий: $scenario" }
  }
}

# Все процессы дерева: корневой и все, в чьей командной строке есть каталог профиля.
function Get-Tree([string]$profileKey, [int]$rootPid) {
  $ids = @()
  foreach ($p in (Get-CimInstance Win32_Process)) {
    if ($p.ProcessId -eq $rootPid) { $ids += $p.ProcessId; continue }
    if ($p.CommandLine -and ($p.CommandLine.IndexOf($profileKey, [StringComparison]::OrdinalIgnoreCase) -ge 0)) { $ids += $p.ProcessId }
  }
  if ($ids.Count -eq 0) { return @() }
  return @(Get-Process -Id $ids -ErrorAction SilentlyContinue)
}

function Measure-Tree([string]$profileKey, [int]$rootPid) {
  $t0 = @{}
  foreach ($p in (Get-Tree $profileKey $rootPid)) { try { $t0[$p.Id] = $p.TotalProcessorTime.TotalSeconds } catch {} }
  Start-Sleep -Seconds $CpuWindowSeconds
  $procs = Get-Tree $profileKey $rootPid
  $cpuDelta = 0.0
  $ws = 0L; $priv = 0L
  foreach ($p in $procs) {
    $ws += $p.WorkingSet64
    $priv += $p.PrivateMemorySize64
    try { if ($t0.ContainsKey($p.Id)) { $cpuDelta += ($p.TotalProcessorTime.TotalSeconds - $t0[$p.Id]) } } catch {}
  }
  [pscustomobject]@{
    Processes = $procs.Count
    WorkingSetMB = [math]::Round($ws / 1MB, 1)
    PrivateMB = [math]::Round($priv / 1MB, 1)
    CpuPct = [math]::Round(100.0 * $cpuDelta / $CpuWindowSeconds / $Cores, 2)
  }
}

function Stop-Tree([string]$profileKey, [int]$rootPid) {
  foreach ($p in (Get-Tree $profileKey $rootPid)) { try { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } catch {} }
}

$exePath = Find-Exe $Browser
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$label = if ($Browser -eq 'nexo' -and $Mode) { "nexo-$Mode" } else { $Browser }
$csv = Join-Path $OutDir "bench-$label.csv"
$rows = @()
$version = if ($Browser -eq 'nexo') { 'nexo' } else { (Get-Item $exePath).VersionInfo.ProductVersion }
Write-Host "Браузер: $label ($exePath) версия: $version; ядер CPU: $Cores"

foreach ($scenario in $Scenarios) {
  for ($run = 1; $run -le $Runs; $run++) {
    $profile = Join-Path ([IO.Path]::GetTempPath()) ("bench-$label-$scenario-$run-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Force -Path $profile | Out-Null
    $urls = @(Urls-For $scenario)
    if ($Browser -eq 'nexo') {
      $launchArgs = @("--data-dir=`"$profile`"")
      if ($Mode) { $launchArgs += "--mode=$Mode" }
    } else {
      $launchArgs = @("--user-data-dir=`"$profile`"", '--no-first-run', '--no-default-browser-check')
    }
    $launchArgs += $urls

    $sw = [Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $exePath -ArgumentList $launchArgs -PassThru
    $startupMs = -1
    while ($sw.Elapsed.TotalSeconds -lt 60) {
      $proc.Refresh()
      if ($proc.HasExited) { break }
      if ($proc.MainWindowHandle -ne 0) { $startupMs = [int]$sw.ElapsedMilliseconds; break }
      Start-Sleep -Milliseconds 50
    }
    if ($proc.HasExited) { Write-Warning "$label/$scenario/$run: процесс завершился сразу после запуска"; continue }

    Start-Sleep -Seconds $SettleSeconds
    if ($scenario -eq 'idle10') { Start-Sleep -Seconds $IdleSeconds }
    $m = Measure-Tree $profile $proc.Id

    $row = [pscustomobject]@{
      Browser = $label; Scenario = $scenario; Run = $run; StartupMs = $startupMs
      Processes = $m.Processes; WorkingSetMB = $m.WorkingSetMB; PrivateMB = $m.PrivateMB; CpuPct = $m.CpuPct
    }
    $rows += $row
    $row | Format-Table -HideTableHeaders | Out-String | Write-Host

    Stop-Tree $profile $proc.Id
    Start-Sleep -Seconds 3
    Remove-Item -Recurse -Force $profile -ErrorAction SilentlyContinue
  }
}

$rows | Export-Csv -NoTypeInformation -Encoding UTF8 $csv
Write-Host "`nСырые данные: $csv"

function Median($values) {
  $s = @($values | Sort-Object)
  if ($s.Count -eq 0) { return $null }
  $mid = [int][math]::Floor($s.Count / 2)
  if ($s.Count % 2 -eq 1) { return $s[$mid] } else { return [math]::Round(($s[$mid - 1] + $s[$mid]) / 2, 1) }
}

Write-Host "`nМедиана по прогонам ($label):"
$summary = foreach ($g in ($rows | Group-Object Scenario)) {
  [pscustomobject]@{
    Scenario = $g.Name; Runs = $g.Count
    StartupMs = Median ($g.Group | ForEach-Object { $_.StartupMs })
    Processes = Median ($g.Group | ForEach-Object { $_.Processes })
    WorkingSetMB = Median ($g.Group | ForEach-Object { $_.WorkingSetMB })
    PrivateMB = Median ($g.Group | ForEach-Object { $_.PrivateMB })
    CpuPct = Median ($g.Group | ForEach-Object { $_.CpuPct })
  }
}
$summary | Format-Table -AutoSize
$summary | Export-Csv -NoTypeInformation -Encoding UTF8 (Join-Path $OutDir "summary-$label.csv")
