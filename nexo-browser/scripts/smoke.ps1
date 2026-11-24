# Дымовой тест на Windows: запускается ли nexo.exe, создаёт ли окно и процессы WebView2, не падает ли за 20 секунд.
# Делает скриншот экрана (на сервере CI он может оказаться пустым — это не ошибка браузера).
param([string]$Exe = '.\target\release\nexo.exe', [string]$OutDir = 'smoke-out')
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$profile = Join-Path ([IO.Path]::GetTempPath()) ('nexo-smoke-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force -Path $profile | Out-Null
$ok = $true
function Check([string]$name, [bool]$cond) {
  $s = if ($cond) { 'PASS' } else { 'FAIL' }
  Write-Host "$s  $name"
  if (-not $cond) { $script:ok = $false }
}

$p = Start-Process -FilePath (Resolve-Path $Exe).Path -ArgumentList @("--data-dir=`"$profile`"", 'https://example.com/') -PassThru
Start-Sleep -Seconds 20
$p.Refresh()
Check 'процесс жив через 20 с' (-not $p.HasExited)
Check 'есть главное окно' ($p.MainWindowHandle -ne 0)
Write-Host "Заголовок окна: $($p.MainWindowTitle)"
$tree = @(Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -and $_.CommandLine.IndexOf($profile, [StringComparison]::OrdinalIgnoreCase) -ge 0 })
Write-Host "Процессов WebView2 с нашим профилем: $($tree.Count)"
Check 'запущены процессы WebView2' ($tree.Count -ge 2)
Check 'создан профиль WebView2' (Test-Path (Join-Path $profile 'WebView2'))
Check 'создана база данных' (Test-Path (Join-Path $profile 'nexo.db'))

try {
  Add-Type -AssemblyName System.Windows.Forms, System.Drawing
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
  $bmp.Save((Join-Path $OutDir 'screen.png'))
} catch { Write-Host "Скриншот не получен: $_" }

foreach ($x in $tree) { try { Stop-Process -Id $x.ProcessId -Force -ErrorAction SilentlyContinue } catch {} }
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
if (-not $ok) { exit 1 }
