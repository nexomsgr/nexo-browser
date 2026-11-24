# Проверка сборки Nexo Browser на Windows. Запуск из корня проекта:
#   powershell -ExecutionPolicy Bypass -File scripts\check-windows.ps1
# Результат (весь вывод) сохраняется в build-log.txt — пришлите этот файл.
$ErrorActionPreference = "Continue"
$log = "build-log.txt"
"== rustc / cargo ==" | Tee-Object $log
rustc --version 2>&1 | Tee-Object $log -Append
cargo --version 2>&1 | Tee-Object $log -Append
"== cargo build (debug) ==" | Tee-Object $log -Append
cargo build 2>&1 | Tee-Object $log -Append
"== cargo test ==" | Tee-Object $log -Append
cargo test 2>&1 | Tee-Object $log -Append
"== cargo build --release ==" | Tee-Object $log -Append
cargo build --release 2>&1 | Tee-Object $log -Append
if (Test-Path target\release\nexo.exe) {
  $f = Get-Item target\release\nexo.exe
  "nexo.exe: $([math]::Round($f.Length/1MB,2)) MB" | Tee-Object $log -Append
} else { "nexo.exe НЕ собран" | Tee-Object $log -Append }
"Готово. Лог: $log"
