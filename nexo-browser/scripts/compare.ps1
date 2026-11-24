# Сводит summary-*.csv разных браузеров в одну таблицу с разницей относительно Nexo (в процентах).
# Запуск: powershell -ExecutionPolicy Bypass -File scripts\compare.ps1 -Dir bench-out -Base nexo-balanced
param([string]$Dir = 'bench-out', [string]$Base = 'nexo-balanced')
$all = foreach ($f in (Get-ChildItem $Dir -Filter 'summary-*.csv')) {
  $label = $f.BaseName -replace '^summary-', ''
  Import-Csv $f.FullName | ForEach-Object { $_ | Add-Member -NotePropertyName Browser -NotePropertyValue $label -PassThru }
}
foreach ($metric in 'WorkingSetMB', 'PrivateMB', 'CpuPct', 'StartupMs', 'Processes') {
  Write-Host "`n== $metric (медиана; % относительно $Base: минус — меньше, чем у $Base) =="
  foreach ($g in ($all | Group-Object Scenario)) {
    $base = $g.Group | Where-Object { $_.Browser -eq $Base } | Select-Object -First 1
    foreach ($r in $g.Group) {
      $pct = ''
      if ($base -and [double]$base.$metric -gt 0 -and $r.Browser -ne $Base) {
        $pct = '{0:+0.0;-0.0}%' -f (100.0 * ([double]$r.$metric - [double]$base.$metric) / [double]$base.$metric)
      }
      '{0,-8} {1,-16} {2,10} {3}' -f $g.Name, $r.Browser, $r.$metric, $pct
    }
  }
}
