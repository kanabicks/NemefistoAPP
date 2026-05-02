# Релизный workflow

С версии **0.1.3** релизы публикуются автоматически через GitHub Actions
на push тега `v*.*.*`. Auto-updater приложения (`tauri-plugin-updater`)
проверяет endpoint `https://github.com/kanabicks/NemefistoAPP/releases/latest/download/latest.json`
раз в 6 часов и предлагает юзеру обновиться.

## Одноразовая настройка

### 1. GitHub Secrets

В репозитории: **Settings → Secrets and variables → Actions → New repository secret**.

Добавить два секрета:

| Имя | Значение |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | содержимое файла `~/.tauri/nemefisto.key` (открой блокнотом, скопируй ВСЁ) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | оставить пустым (ключ сгенерён без пароля) |

### 2. Public key уже в `tauri.conf.json`

В `plugins.updater.pubkey` лежит публичный ключ из `~/.tauri/nemefisto.key.pub`.
Его трогать не надо — он попадает в bundled NSIS и проверяет подпись `latest.json`
со стороны клиента.

⚠️ **Важно**: если приватный ключ потерян — все существующие установленные
клиенты НЕ смогут получать обновления (подпись новых релизов будет невалидной).
Без бекапа `~/.tauri/nemefisto.key` придётся ставить новый ключ + публиковать
0.X.0 «обновление через ручную скачку» с release-нот.

## Как сделать релиз

```powershell
# 1. Bump версии в трёх местах: package.json, src-tauri/Cargo.toml,
#    src-tauri/tauri.conf.json (поле version).
#    Все три должны совпадать.

# 2. Коммит изменений.
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock
git commit -m "chore: release v0.1.4"

# 3. Создаёшь tag.
git tag v0.1.4 -m "v0.1.4 — описание"

# 4. Пушишь и main, и tag (одним push'ем не получится — push --follow-tags).
git push origin main
git push origin v0.1.4

# CI запускается автоматически. Прогресс — в GitHub → Actions.
# Через ~10-15 минут release появится в GitHub → Releases.
```

## Что делает CI

`.github/workflows/release.yml`:

1. Checkout с полной историей (для CHANGELOG-генерации).
2. Setup Node.js 22, Rust stable, cache cargo + npm.
3. `npm ci`.
4. `npm run prepare-bundle` — собирает `nemefisto-helper.exe` release-сборкой,
   копирует в `src-tauri/binaries/` с triplet-суффиксом.
5. **`tauri-apps/tauri-action@v0`**:
   - вызывает `tauri build` (через `beforeBuildCommand: npm run build`
     прогоняет TypeScript + vite build);
   - bundler собирает NSIS-installer;
   - подписывает .exe ed25519-ключом из `TAURI_SIGNING_PRIVATE_KEY`;
   - генерирует `latest.json` (manifest для updater);
   - создаёт GitHub Release с тегом и публикует assets:
     - `Nemefisto_<VER>_x64-setup.exe`
     - `Nemefisto_<VER>_x64-setup.exe.sig` (подпись)
     - `latest.json`

После этого:
- Юзеры с **0.1.2 и старше** должны скачать NSIS вручную (у них нет updater'а).
- Юзеры с **0.1.3+** через ~10 секунд после старта приложения увидят модалку
  «доступна v X.Y.Z», смогут обновиться одним кликом.

## Настройка `latest.json`

`tauri-action` сам формирует:

```json
{
  "version": "0.1.4",
  "notes": "...",
  "pub_date": "2026-...",
  "platforms": {
    "windows-x86_64": {
      "signature": "...",
      "url": "https://github.com/.../Nemefisto_0.1.4_x64-setup.exe"
    }
  }
}
```

Этот JSON прикреплён к release как asset. Endpoint в `tauri.conf.json`
указывает на `releases/latest/download/latest.json` — GitHub автоматически
редиректит на самый новый release.

## Откат / удаление релиза

Если релиз сломанный:

```powershell
# 1. Снять тег локально и на GitHub.
git tag -d v0.1.4
git push origin :refs/tags/v0.1.4

# 2. Удалить release через UI: github.com/.../releases → Delete.

# 3. (опц.) выпустить новый тег v0.1.5 с фиксом.
```

⚠️ Если юзеры уже получили auto-update — у них установлена сломанная
версия. Хороший пре-релиз практика: сначала тестируем 0.1.4-rc1
вручную, и только потом продвигаем 0.1.4 stable.

## Локальная сборка (без CI)

Если нужно собрать NSIS на своей машине (не для публикации):

```powershell
npm run tauri:bundle
# Результат: src-tauri/target/release/bundle/nsis/Nemefisto_<VER>_x64-setup.exe
```

⚠️ Этот NSIS не подписан (нет ed25519-подписи) — auto-updater откажется
его принимать. Для production нужен только CI-build.

## Тестирование auto-updater'а

После релиза 0.1.3 (текущего):

1. Установи 0.1.3 NSIS вручную.
2. Запусти приложение.
3. Открой Settings → обновления → «проверить сейчас» — должно сказать
   «у вас уже последняя версия».
4. Когда выпустим 0.1.4 (следующий релиз) — подожди ~10 секунд после
   старта, должна всплыть модалка «доступна v 0.1.4». Жми «обновить
   и перезапустить» — приложение скачает новый NSIS (~45 МБ),
   запустит passive-install, перезапустится с обновлённой версией.
