; 0.3.1 / installer file-lock fix.
;
; При обновлении (через auto-updater или вручную скачанный installer)
; `nemefisto-helper.exe` залочен потому что зарегистрирован как
; Windows-service `NemefistoHelper` под SYSTEM. NSIS не может
; перезаписать файл запущенного процесса.
;
; Auto-updater сначала вызывает Tauri-команду `shutdown_helper` (см.
; src/lib/updater.ts), которая просит helper выйти грациозно через SCM
; SERVICE_CONTROL_STOP. После ~1.5с файл уже свободен, и эти хуки
; работают как defensive-резерв.
;
; Для **manual install** (юзер скачал installer и запустил вручную):
; - если запущен как админ → `sc stop` срабатывает, файл освобождается;
; - если без админа → `sc stop` тихо фейлится, и юзер увидит тот же
;   диалог "невозможно открыть файл" что раньше. Не регрессия, но
;   улучшение для самого частого пути (auto-update).

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "Stopping Nemefisto Helper service before update..."
  ; sc stop требует SERVICE_STOP rights на сервис. По умолчанию это
  ; только Administrators/SYSTEM. Без админа просто silently fails —
  ; не падаем на error.
  nsExec::ExecToLog 'sc stop NemefistoHelper'
  ; Ждём чтобы SCM успел маршрутизировать STOP-сигнал и helper-процесс
  ; завершился (закрыл свой image-handle).
  Sleep 1500
  ; Defensive: если sc stop не помог (например, helper висит и не
  ; реагирует на SERVICE_CONTROL_STOP), пробуем kill. Тоже требует
  ; админа на SYSTEM-процесс.
  nsExec::ExecToLog 'taskkill /F /T /IM nemefisto-helper-x86_64-pc-windows-msvc.exe'
  ; 0.3.2: VPN-движки (sing-box / mihomo) могут остаться orphan'ами после
  ; helper-shutdown — kill'им их тоже. Tauri-sidecar запущен под user'ом
  ; (taskkill работает без админа), SYSTEM-spawned требует админ-прав.
  ; Frontend disconnect должен был их остановить нормально, это backup.
  nsExec::ExecToLog 'taskkill /F /T /IM sing-box-x86_64-pc-windows-msvc.exe'
  nsExec::ExecToLog 'taskkill /F /T /IM mihomo-x86_64-pc-windows-msvc.exe'
  Sleep 500
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing Nemefisto Helper service..."
  nsExec::ExecToLog 'sc stop NemefistoHelper'
  Sleep 1500
  nsExec::ExecToLog 'taskkill /F /T /IM nemefisto-helper-x86_64-pc-windows-msvc.exe'
  ; 0.3.2: kill VPN-движки если ещё живы
  nsExec::ExecToLog 'taskkill /F /T /IM sing-box-x86_64-pc-windows-msvc.exe'
  nsExec::ExecToLog 'taskkill /F /T /IM mihomo-x86_64-pc-windows-msvc.exe'
  Sleep 500
  ; После stop сервис всё ещё зарегистрирован в SCM. При полной
  ; деинсталляции удаляем чтобы не оставлять "висящую" запись.
  nsExec::ExecToLog 'sc delete NemefistoHelper'
!macroend
