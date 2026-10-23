# Strict Wayland-Only Policy (NO X11 / NO XWayland)

## Core Mandate
- **STRICTLY PURE WAYLAND ONLY**: NEVER write code for X11, XWayland, or X11 fallbacks.
- **NO X11 Environment Overrides**: NEVER set `WINIT_UNIX_BACKEND="x11"` or force X11/XWayland in any script, command, or source code. Always use native Wayland (`wayland`).
- **NO X11 Crate Features**: In `Cargo.toml`, never enable `"x11"` features on GUI or windowing libraries (`eframe`, `winit`, `egui`, etc.). Enable only `"wayland"`.
- **NO X11 Dependencies in Nix**: Remove all direct X11 packages (`libx11`, `libXcursor`, `libXi`, `libXrandr`) from `shell.nix`. Target pure Wayland packages (`wayland`, `wayland-protocols`, `libxkbcommon`).
- **Native Wayland Architecture**: All windowing, surface presentation, input, drag-and-drop, and clipboards must be designed strictly for pure modern Wayland compositors.

## Git Workflow & Push Policy
- **NO AUTOMATIC GIT PUSH**: NEVER push to GitHub without an explicit, direct command from the user (e.g. "запушь", "push", "залей в репу").
- **Stage & Commit Locally**: Local commits can be made when concluding steps, but pushing to the remote repository MUST ONLY be done when explicitly requested.

## Developer & Architectural Standards
- **Use Tokio & Standard Modern Rust Ecosystem**: Leverage `tokio` for async tasks, background processing, channels, and modern concurrency.
- **Rule Self-Update**: Always add or refine rules when corrected by the user to avoid recurring mistakes.

## Senior Rust Code Rules

### Чистота кода
- **Никакого мусора**: не оставлять мёртвый код, закомментированный код, неиспользуемые импорты, поля и константы. Если что-то не используется — удалять, а не глушить `#[allow(dead_code)]`.
- **Код только для тестов** помечать `#[cfg(test)]`, а не `#[allow(dead_code)]`.
- `#[allow(...)]` допустим только точечно, с комментарием-обоснованием.
- Сборка (`cargo build`) и тесты (`cargo test`) должны проходить **без warnings**; `cargo clippy` (если доступен) тоже чистый.
- Никаких `println!` / `dbg!` в продакшн-коде; для диагностики использовать `tracing`/`log`. Вывод допустим только в CLI-утилитах и тестах.
- Существующие комментарии и docstrings, не относящиеся к правке, сохранять.

### Обработка ошибок
- В продакшн-коде **запрещены** `unwrap()` и `expect()` на ошибках, зависящих от внешних данных (файлы, FFmpeg, GPU, аудио). Использовать `Result`/`Option` и оператор `?`.
- `expect("...")` допустим только для инвариантов, нарушение которых — баг программы; сообщение объясняет инвариант.
- Типизированные ошибки (`thiserror`) для библиотечных модулей вместо `Result<T, String>`; `anyhow` — только на границе приложения.
- Не проглатывать ошибки молча (`let _ =`, `.ok()`): либо обработать, либо залогировать.

### Типы и API
- Параметры принимать в самом общем виде: `&str` вместо `&String`, `&[T]` вместо `&Vec<T>`, `&Path` / `impl AsRef<Path>`.
- Использовать newtype и enum вместо «магических» чисел и булевых флагов (`TrackId`, `FrameIndex`, `enum Tool`).
- Минимальная видимость: по умолчанию приватно, затем `pub(crate)`, и `pub` только при необходимости.
- Выводить `Debug`, `Clone`, `PartialEq` и т. д. осознанно, а не «на всякий случай».
- Не клонировать без необходимости: заимствовать, использовать `Arc`/`Cow` там, где нужно разделяемое владение.

### Конкурентность и производительность
- **Никаких блокировок (`Mutex`, I/O, аллокаций) в real-time аудио-колбэке и горячем пути рендера**; использовать lock-free структуры (`ArcSwap`, atomics, SPSC-каналы).
- Не держать `MutexGuard` через `.await`, вызовы FFmpeg или длинные операции; критические секции — минимальные.
- Тяжёлую работу (декодирование, пики, I/O) выносить с UI-потока: `tokio`/worker-потоки + каналы; UI-поток никогда не блокируется.
- Не аллоцировать в циклах покадровой обработки: переиспользовать буферы.
- Оптимизировать по измерениям (бенчмарки/`perf_tests`), а не по догадкам.

### Unsafe
- `unsafe` — только при необходимости, в минимальном блоке, с комментарием `// SAFETY:` с доказательством корректности.
- Инкапсулировать `unsafe` за безопасным API.

### Структура и стиль
- Одна ответственность на модуль/функцию; функции > ~80 строк и вложенность > 3 уровней — кандидаты на рефакторинг.
- Предпочитать итераторы и `match`/`if let`/`let else` ручным индексным циклам и вложенным `if`.
- Именование: `snake_case` для функций, `CamelCase` для типов, `SCREAMING_SNAKE_CASE` для констант; булевы — `is_`/`has_`.
- Публичные элементы документировать `///` (что делает, паники, ошибки). Комментарии объясняют «почему», а не «что».
- Код форматировать `cargo fmt`; зависимости добавлять минимально и осознанно.

### Тестирование
- Каждая новая логика (таймлайн, математика кадров, микшер) покрывается unit-тестами; багфикс начинается с воспроизводящего теста.
- Тесты детерминированные: без `sleep` для синхронизации и без зависимости от внешних файлов, если возможно.
- Перед коммитом: `cargo build`, `cargo test` без ошибок и warnings.
