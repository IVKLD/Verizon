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

### Стандарты видеоконвейера, Render Cache и Egui UI (Защита от регрессий)
- **Строгая чистота Render Cache (Zero Cache Poisoning)**:
  - В `RenderCache` разрешено записывать **только точные декодированные кадры** (`src_f == target_src`), поступившие из точного декодирования (`newly_cached_frames` или `last_exact_frame == target_src && last_decoded_frame == target_src`).
  - Приближённые кадры из `scrub_cache` (`cached_f != target_frame`) и ключевые превью-кадры (`raw_frame_num < 0`) **никогда** не должны попадать в `RenderCache`, перезаписывать `last_exact_frame` или подменять `last_displayed_image` для соседних кадров (`|presented_src - target_src| <= 1` запрещено для незавершённого сика).
  - При попадании в `RenderCache` (`cached_image.is_some()`) фоновый `poll_frames` не имеет права перетирать экранную текстуру старыми кадрами из очереди сика, а `present_playback_frame` обязан проверять `Arc::ptr_eq` против `last_displayed_image`, чтобы не загружать одну и ту же 8 МБ текстуру в GPU каждый кадр.
- **Запрет фальшивых кадров разгона и мигания при Play/Seek**:
  - В `DecoderCommand::Play` кадры разгона от ключевого кадра до точки старта (`f_num < start_playback_frame`) **никогда** не должны добавляться в `frame_queue` воспроизведения, не должны завершать пре-ролл (`playback_ready = true`) раньше времени и не должны выводиться на экран вместо целевого кадра.
  - В `DecoderCommand::Seek` условие `can_decode_forward` требует строго `target_pts > current_pts` (при `target_pts == current_pts` кадр уже вычитан из декодера).
  - В фоновом `BackgroundCacheEngine::run_cache_worker` сопоставление кадров в основном цикле должно быть точным (`target_map.remove(&cur_src_f)`), чтобы кадр `N-1` никогда не «крал» слот кадра `N`.
  - Фоновый `Smart Cache` вокруг плейхеда обязан ограничивать окно предзагрузки лимитом `RenderCacheManager::max_frames`, исключая бесконечное вытеснение (LRU thrashing) на длинных видео.
- **Стандартизированный ввод и вёрстка в Egui**:
  - **Зум с зажатым `Ctrl` (`Ctrl + MouseWheel`)**: в `egui` при зажатом `Ctrl` событие колеса мыши преобразуется в `i.zoom_delta()`, а `i.smooth_scroll_delta` обнуляется. Любой масштабируемый компонент (Viewer, Timeline) обязан считывать **и** `i.zoom_delta()`, **и** сырые события `egui::Event::MouseWheel` из `i.events`, а проверку наведения курсора делать через `rect.contains(pointer_pos)`, а не `bg_resp.hovered()`, так как дочерние интерактивные слои (например, Transform OSC) перехватывают `hovered()`.
  - **Запрет `ui.set_height` / `ui.set_width` в начале контейнера**: в `egui` вызов `ui.set_height(h)` в начале вертикального `Ui` (`TopDown`) или `ui.set_width(w)` в начале горизонтального `Ui` (`LeftToRight`) сдвигает внутренний `cursor` в конец области до добавления виджетов, ломая вёрстку и выталкивая контент за пределы окна. Для жёстких размеров использовать `ui.allocate_ui_with_layout`, `ScrollArea::max_height` или вызывать `set_min_size` в конце замыкания.
  - **Модальные диалоговые окна**: каждое диалоговое окно обязано иметь фиксированную 3-секционную структуру на детерминированных непересекающихся прямоугольниках (`header_outer`, `body_outer`, `footer_outer`) с адаптацией под размер экрана (`ctx.content_rect()`), видимой кнопкой закрытия `✕` в правом верхнем углу и закрытием по `✕` и `Esc`. При создании дочерних областей через `ui.new_child(egui::UiBuilder::new()...)` **обязательно** указывать уникальный `.id_salt("...")`, чтобы виджеты в разных зонах никогда не получали одинаковый `auto_id`.
  - **Интерактивный Transform OSC во Viewer**: экран проекта `16:9` в режиме `Fit` обязан иметь студийные отступы внутри области монитора, чтобы все 8 ручек масштабирования и верхняя ручка вращения (`p_rot`) никогда не обрезались краем контейнера; само видео клипируется по `screen_rect`, а векторная рамка и ручки Transform рисуются поверх всего контейнера монитора с масштабированием 1:1 за курсором.
