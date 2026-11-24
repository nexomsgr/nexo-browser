# Как мерить Nexo против Chrome / Edge / Brave

Скрипт `scripts/benchmark.ps1` запускает каждый браузер с **чистым временным профилем**, ждёт 30 с и измеряет
**всё дерево процессов** (число процессов, Working Set, Private Bytes, CPU за 20 с) и время до появления окна.
Дерево определяется по каталогу профиля в командной строке процесса, поэтому метрика одинакова для всех.

## Сценарии
| Сценарий | Что открыто |
|---|---|
| `blank` | одна пустая вкладка |
| `one` | одна вкладка google.com |
| `five` / `ten` | 5 / 10 вкладок https://www.wikipedia.org/ |
| `idle10` | 10 вкладок, затем простой 6 минут (проверка сна вкладок) |
| YouTube | вручную: откройте ролик, включите воспроизведение на 2 минуты, затем смотрите `nexo://performance` (Nexo) или Диспетчер задач |

## Запуск у себя
```powershell
powershell -ExecutionPolicy Bypass -File scripts\benchmark.ps1 -Browser nexo -Exe .\target\release\nexo.exe -Mode balanced
powershell -ExecutionPolicy Bypass -File scripts\benchmark.ps1 -Browser nexo -Exe .\target\release\nexo.exe -Mode memory
powershell -ExecutionPolicy Bypass -File scripts\benchmark.ps1 -Browser chrome
powershell -ExecutionPolicy Bypass -File scripts\benchmark.ps1 -Browser brave
powershell -ExecutionPolicy Bypass -File scripts\compare.ps1 -Dir bench-out -Base nexo-balanced
```
Для честности: закройте другие программы, не трогайте компьютер во время замера, подключение к сети одно и то же,
для всех браузеров одинаковое число прогонов (рекомендуется ≥ 3, берётся медиана).

## Как читать результаты
* **Working Set** включает общие страницы (DLL, общая память) — завышает суммарное потребление при множестве процессов.
* **Private Bytes** — зафиксированная приватная память процессов, ближе к реальной «стоимости» браузера.
* Это не «ОЗУ браузера в Диспетчере задач» в точности, но метрика у всех браузеров одна и та же.
* Результаты зависят от компьютера, версии браузера и сайтов — сравнивайте только прогоны на одной машине.
