//! Приложение: окно tao + WebView2 с общим профилем (один браузерный процесс):
//! * `chrome` — собственный интерфейс (панель вкладок, адресная строка): занимает всё окно и рисует градиент;
//! * `views`  — по одному WebView2 на вкладку, лежат поверх `chrome` с отступами. Показан только активный;
//!   скрытые WebView2 браузер сам троттлит, а закрытая вкладка полностью освобождает свой WebView2.
//!
//! Сайты не получают IPC и не могут управлять интерфейсом или вкладками. Всё, что приходит от
//! интерфейса, проходит через `UiMessage::parse` и проверку источника.

use crate::browser::navigation::{
    address_text, custom_protocol_url, internal_load_url, is_protocol_origin, parse_load_url,
    resolve_input, security_of, to_display_url, InternalPage, Target, PAGE_SCHEME, UI_SCHEME,
};
use crate::browser::tab::{TabId, TabInfo, TabState};
use crate::browser::tab_manager::TabManager;
use crate::browser::webview_ops;
use crate::browser::zoom;
use crate::command::Command;
use crate::downloads::Downloads;
use crate::internal_pages::data;
use crate::internal_pages::performance::{build_report, free_memory_notice, ReportInput};
use crate::memory::monitor::{self, CpuTracker};
use crate::memory::policy::PerformanceMode;
use crate::memory::sleeper::{self, SleepAction};
use crate::shortcuts::{self, Chord};
use crate::storage::session::SessionTab;
use crate::storage::settings::{Settings, ThemeChoice};
use crate::storage::Db;
use crate::ui::assets::{self, Asset};
use crate::ui::ipc::{TabView, UiMessage, UiState};
use crate::ui::theme::{self, Theme};
use crate::util::{now_ms, Throttle};
use anyhow::{anyhow, Context, Result};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Fullscreen, Window, WindowBuilder};
use wry::http::{Request, Response};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

/// Высота панели интерфейса (вкладки 46 + инструменты 46) и отступ вокруг страницы, логические px.
const CHROME_H: f64 = 92.0;
const GUTTER: f64 = 8.0;
/// Предохранитель от бесконечного открытия вкладок (всплывающие окна, ошибки сайтов).
const MAX_TABS: usize = 100;

/// Отслеживает пользовательский ввод на странице: по нему решаем, что вкладку нельзя усыплять
/// (в ней могут быть несохранённые данные). Учитываются только настоящие (isTrusted) события
/// верхнего документа; ввод внутри iframe не отслеживается.
const DIRTY_TRACKER_JS: &str =
    "(()=>{if(window.__nexoDirty!==undefined)return;window.__nexoDirty=false;\
addEventListener('input',e=>{if(e.isTrusted)window.__nexoDirty=true;},true);})();";
/// Флаг командной строки: открыть приватное окно.
const PRIVATE_FLAG: &str = "--private";
const PROBE_JS: &str = "window.__nexoDirty===true";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SleepKind {
    Suspend,
    Discard,
}

/// События, которые обработчики WebView отправляют в главный цикл.
#[derive(Debug, Clone)]
enum UserEvent {
    /// Команда от собственного интерфейса.
    Ui(UiMessage),
    /// Команда от внутренней страницы nexo:// конкретной вкладки (уже отфильтрована `allowed_from_page`).
    Page(TabId, UiMessage),
    /// Началась навигация верхнего уровня во вкладке.
    NavStarted(TabId, String),
    /// URL вкладки изменился без загрузки страницы (history.pushState и т. п.). Источник — событие
    /// WebView2 `SourceChanged`, которое подключается только на Windows.
    #[cfg_attr(not(windows), allow(dead_code))]
    UrlChanged(TabId, String),
    LoadStarted(TabId),
    LoadFinished(TabId),
    Title(TabId, String),
    /// Страница запросила новое окно (`window.open`, `target=_blank`).
    NewWindow(String),
    /// Интерфейс загрузился и готов принимать состояние.
    ChromeReady,
    /// Результат проверки «есть ли во вкладке несохранённый ввод» перед сном.
    InputProbe(TabId, bool, SleepKind),
    /// WebView2 ответил на `TrySuspend`.
    SuspendDone(TabId, bool),
    /// Команда от горячих клавиш, перехваченных в странице сайта (событие WebView2 `AcceleratorKeyPressed`).
    #[cfg_attr(not(windows), allow(dead_code))]
    Cmd(Command),
    /// Страница вошла в полноэкранный режим или вышла из него (например, видео на YouTube).
    #[cfg_attr(not(windows), allow(dead_code))]
    HtmlFullscreen(TabId, bool),
    #[cfg_attr(not(windows), allow(dead_code))]
    Download(DownloadEvent),
}

/// События загрузок из WebView2. Номер загрузки назначает обработчик события.
#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
enum DownloadEvent {
    Started {
        tab: TabId,
        id: u64,
        url: String,
        path: String,
        total: Option<u64>,
    },
    Progress {
        id: u64,
        received: u64,
        total: Option<u64>,
    },
    Ended {
        id: u64,
        ok: bool,
    },
}

struct App {
    window: Window,
    chrome: WebView,
    /// Живые WebView2 вкладок. Вкладка без записи здесь (выгруженная) — шаг 5.
    views: HashMap<TabId, WebView>,
    tabs: TabManager,
    db: Option<Db>,
    settings: Settings,
    /// Разрешает один переход на file:// — только после явного ввода пользователя.
    allow_file_nav: Arc<AtomicBool>,
    proxy: EventLoopProxy<UserEvent>,
    theme: Theme,
    theme_css: String,
    popup_throttle: Throttle,
    /// Аргументы WebView2, с которыми запущен браузер (общие для всех WebView).
    launch_args: String,
    cpu: CpuTracker,
    /// Вкладки, для которых уже отправлена проверка/приостановка и ответ ещё не пришёл.
    sleep_pending: HashSet<TabId>,
    next_tick: Instant,
    downloads: Downloads,
    dl_throttle: Throttle,
    /// Приватное окно: временный профиль, ничего не пишется в историю, закладки и сессию.
    private: bool,
    /// Каталог временного профиля приватного окна (удаляется при закрытии).
    private_dir: Option<PathBuf>,
    user_fullscreen: bool,
    html_fullscreen: bool,
    history_query: String,
    /// Есть ли текущий адрес в закладках — чтобы не ходить в базу при каждом обновлении интерфейса.
    bookmark_cache: Option<(String, bool)>,
    // Должен быть последним: WebContext обязан пережить все WebView (поля освобождаются по порядку).
    ctx: WebContext,
}

pub fn run() -> Result<()> {
    let private = std::env::args().skip(1).any(|a| a == PRIVATE_FLAG);
    let base_dir = data_dir();
    std::fs::create_dir_all(&base_dir)
        .with_context(|| format!("создание {}", base_dir.display()))?;

    // Приватное окно: отдельный временный профиль WebView2 и база только в памяти.
    let private_dir =
        private.then(|| base_dir.join(format!("private-{}-{}", std::process::id(), now_ms())));
    let data_dir = private_dir.clone().unwrap_or_else(|| base_dir.clone());
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("создание {}", data_dir.display()))?;

    // Сбой базы не должен мешать запуску браузера: работаем с настройками по умолчанию.
    let db = if private {
        Db::open_in_memory().ok()
    } else {
        Db::open(&base_dir.join("nexo.db")).ok()
    };
    let mut settings = db
        .as_ref()
        .and_then(|d| d.load_settings().ok())
        .unwrap_or_default();
    if private {
        // Настройки берём из обычного профиля, но память/сон — как есть; сохранять их из приватного окна нельзя.
        if let Ok(main) = Db::open(&base_dir.join("nexo.db")) {
            settings = main.load_settings().unwrap_or_default();
        }
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title(if private {
            "Nexo Browser — приватное окно"
        } else {
            "Nexo Browser"
        })
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(640.0, 420.0))
        .build(&event_loop)
        .map_err(|e| anyhow!("не удалось создать окно: {e}"))?;

    let (th, dark) = pick_theme(settings.theme, &window);
    style_title_bar(&window, th, dark);
    let theme_css = th.css_vars();

    let launch_args = settings.performance_mode.browser_args();
    let mut ctx = WebContext::new(Some(data_dir.join("WebView2")));
    let chrome = build_chrome(&mut ctx, &window, &proxy, &theme_css, th, &launch_args)?;
    let startup_url = startup_target(&settings).load_url();

    let mut app = App {
        window,
        chrome,
        views: HashMap::new(),
        tabs: TabManager::new(),
        db,
        settings,
        allow_file_nav: Arc::new(AtomicBool::new(false)),
        proxy,
        theme: *th,
        theme_css,
        popup_throttle: Throttle::new(400),
        launch_args,
        cpu: CpuTracker::new(
            std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1),
        ),
        sleep_pending: HashSet::new(),
        next_tick: Instant::now(),
        downloads: Downloads::new(),
        dl_throttle: Throttle::new(250),
        private,
        private_dir,
        user_fullscreen: false,
        html_fullscreen: false,
        history_query: String::new(),
        bookmark_cache: None,
        ctx,
    };
    app.schedule_tick();
    // Ошибка создания первой вкладки (например, нет WebView2 Runtime) доходит до диалога show_fatal.
    let explicit_url = cli_url().is_some();
    let restored =
        !private && !explicit_url && app.settings.restore_session && app.restore_session();
    if !restored {
        app.open_tab(&startup_url, true)?;
    }

    event_loop.run(move |event, _, control_flow| {
        *control_flow = app.wait_mode();
        match event {
            // Периодическая проверка усыпления. Между тиками процесс спит и не расходует CPU.
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                app.tick();
                app.schedule_tick();
                *control_flow = app.wait_mode();
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                app.shutdown();
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            }
            | Event::WindowEvent {
                event: WindowEvent::ScaleFactorChanged { .. },
                ..
            } => {
                app.layout();
            }
            Event::UserEvent(ev) => {
                if app.handle(ev) {
                    *control_flow = ControlFlow::Exit;
                } else {
                    *control_flow = app.wait_mode();
                }
            }
            _ => {}
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Построение WebView
// ---------------------------------------------------------------------------------------------

fn build_chrome(
    ctx: &mut WebContext,
    window: &Window,
    proxy: &EventLoopProxy<UserEvent>,
    theme_css: &str,
    th: &Theme,
    browser_args: &str,
) -> Result<WebView> {
    let css = theme_css.to_string();
    let (p_ipc, p_ready) = (proxy.clone(), proxy.clone());

    let builder = WebViewBuilder::new_with_web_context(ctx)
        .with_background_color(rgba(th.bg_start))
        .with_custom_protocol(UI_SCHEME.to_string(), move |_id, req| {
            respond(assets::chrome_asset(req.uri().path(), &css))
        })
        .with_url(custom_protocol_url(UI_SCHEME, "index.html"))
        .with_ipc_handler(move |req: Request<String>| {
            if !is_protocol_origin(&req.uri().to_string(), UI_SCHEME) {
                return;
            }
            if let Some(msg) = UiMessage::parse(req.body()) {
                let _ = p_ipc.send_event(UserEvent::Ui(msg));
            }
        })
        // Интерфейс никогда не уходит со своей страницы и не открывает новых окон.
        .with_navigation_handler(|url| is_protocol_origin(&url, UI_SCHEME))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_on_page_load_handler(move |ev, _| {
            if matches!(ev, PageLoadEvent::Finished) {
                let _ = p_ready.send_event(UserEvent::ChromeReady);
            }
        })
        .with_hotkeys_zoom(false)
        .with_devtools(cfg!(debug_assertions));
    let builder = with_browser_args(builder, browser_args);

    builder
        .build_as_child(window)
        .map_err(|e| anyhow!("не удалось создать WebView2 интерфейса: {e}"))
}

/// WebView2 одной вкладки. Все обработчики помечают события идентификатором вкладки.
#[allow(clippy::too_many_arguments)]
fn build_content(
    ctx: &mut WebContext,
    window: &Window,
    proxy: &EventLoopProxy<UserEvent>,
    theme_css: &str,
    th: &Theme,
    id: TabId,
    initial_url: &str,
    allow_file_nav: &Arc<AtomicBool>,
    visible: bool,
    browser_args: &str,
) -> Result<WebView> {
    let css = theme_css.to_string();
    let p_ipc = proxy.clone();
    let p_nav = proxy.clone();
    let p_win = proxy.clone();
    let p_title = proxy.clone();
    let p_load = proxy.clone();
    let file_flag = allow_file_nav.clone();

    let builder = WebViewBuilder::new_with_web_context(ctx)
        .with_visible(visible)
        .with_background_color(rgba(th.panel))
        .with_custom_protocol(PAGE_SCHEME.to_string(), move |_id, req| {
            respond(assets::page_asset(req.uri().path(), &css))
        })
        .with_url(initial_url)
        .with_initialization_script(DIRTY_TRACKER_JS)
        // IPC принимаем только от наших внутренних страниц и только навигацию (строка поиска «Новой вкладки»).
        .with_ipc_handler(move |req: Request<String>| {
            if !is_protocol_origin(&req.uri().to_string(), PAGE_SCHEME) {
                return;
            }
            if let Some(msg) = UiMessage::parse(req.body()).filter(UiMessage::allowed_from_page) {
                let _ = p_ipc.send_event(UserEvent::Page(id, msg));
            }
        })
        .with_navigation_handler(move |url| {
            if !navigation_allowed(&url, &file_flag) {
                return false;
            }
            let _ = p_nav.send_event(UserEvent::NavStarted(id, url));
            true
        })
        // Новое окно открываем как новую вкладку; фильтр спама и ограничения — в App::handle.
        .with_new_window_req_handler(move |url, _| {
            if url.starts_with("http://") || url.starts_with("https://") {
                let _ = p_win.send_event(UserEvent::NewWindow(url));
            }
            NewWindowResponse::Deny
        })
        .with_document_title_changed_handler(move |title| {
            let _ = p_title.send_event(UserEvent::Title(id, title));
        })
        .with_on_page_load_handler(move |ev, _| {
            let _ = p_load.send_event(match ev {
                PageLoadEvent::Started => UserEvent::LoadStarted(id),
                PageLoadEvent::Finished => UserEvent::LoadFinished(id),
            });
        })
        .with_hotkeys_zoom(true)
        .with_devtools(cfg!(debug_assertions));
    let builder = with_browser_args(builder, browser_args);

    builder
        .build_as_child(window)
        .map_err(|e| anyhow!("не удалось создать WebView2 страницы: {e}"))
}

/// Аргументы процессов WebView2. Окружение WebView2 общее для всех WebView, поэтому аргументы
/// должны быть одинаковыми у всех — иначе создание следующего WebView завершится ошибкой.
#[cfg(windows)]
fn with_browser_args<'a>(b: WebViewBuilder<'a>, args: &str) -> WebViewBuilder<'a> {
    use wry::WebViewBuilderExtWindows;
    b.with_additional_browser_args(args)
}

#[cfg(not(windows))]
fn with_browser_args<'a>(b: WebViewBuilder<'a>, _args: &str) -> WebViewBuilder<'a> {
    b
}

/// Какие переходы верхнего уровня разрешены. `javascript:`, `data:`, внешние протоколы
/// (mailto:, ms-*: …) блокируются; file:// — только после явного ввода пользователем.
fn navigation_allowed(url: &str, allow_file: &AtomicBool) -> bool {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://")
        || lower.starts_with("about:blank")
        || lower.starts_with("blob:")
    {
        return true;
    }
    if lower.starts_with("http://") {
        return true;
    }
    if lower.starts_with("file:") {
        return allow_file.swap(false, Ordering::SeqCst);
    }
    // Внутренние страницы на Linux-форме nexo://localhost/… (только для проверки сборки).
    is_protocol_origin(url, PAGE_SCHEME)
}

fn respond(asset: Option<Asset>) -> Response<Cow<'static, [u8]>> {
    let built = match asset {
        Some(a) => Response::builder()
            .status(200)
            .header("Content-Type", a.mime)
            .header("Cache-Control", "no-store")
            .header("X-Content-Type-Options", "nosniff")
            .body(Cow::Owned(a.body)),
        None => Response::builder()
            .status(404)
            .body(Cow::Borrowed(&b"not found"[..])),
    };
    built.unwrap_or_default()
}

fn rgba(c: theme::Rgb) -> (u8, u8, u8, u8) {
    (c.0, c.1, c.2, 255)
}

// ---------------------------------------------------------------------------------------------
// Поведение
// ---------------------------------------------------------------------------------------------

impl App {
    fn is_fullscreen(&self) -> bool {
        self.user_fullscreen || self.html_fullscreen
    }

    fn content_rect(&self) -> Rect {
        let sf = self.window.scale_factor();
        let size: LogicalSize<f64> = self.window.inner_size().to_logical(sf);
        let (w, h) = (size.width.max(1.0), size.height.max(1.0));
        if self.is_fullscreen() {
            // В полноэкранном режиме страница занимает всё окно, без панели и отступов.
            return Rect {
                position: LogicalPosition::new(0.0, 0.0).into(),
                size: LogicalSize::new(w, h).into(),
            };
        }
        Rect {
            position: LogicalPosition::new(GUTTER, CHROME_H).into(),
            size: LogicalSize::new(
                (w - 2.0 * GUTTER).max(1.0),
                (h - CHROME_H - GUTTER).max(1.0),
            )
            .into(),
        }
    }

    fn layout(&self) {
        let sf = self.window.scale_factor();
        let size: LogicalSize<f64> = self.window.inner_size().to_logical(sf);
        let _ = self.chrome.set_visible(!self.is_fullscreen());
        let _ = self.chrome.set_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(size.width.max(1.0), size.height.max(1.0)).into(),
        });
        for view in self.views.values() {
            let _ = view.set_bounds(self.content_rect());
        }
    }

    fn apply_fullscreen(&mut self) {
        let want = self.is_fullscreen();
        self.window
            .set_fullscreen(want.then_some(Fullscreen::Borderless(None)));
        self.layout();
        self.show_only_active();
    }

    /// Показывает WebView2 активной вкладки, прячет остальные и отдаёт фокус странице.
    fn show_only_active(&self) {
        let active = self.tabs.active_id();
        for (id, view) in &self.views {
            let _ = view.set_visible(Some(*id) == active);
        }
        self.apply_memory_levels();
        if let Some(view) = active.and_then(|id| self.views.get(&id)) {
            let _ = view.focus();
        }
    }

    /// Режим «Экономия памяти»: скрытым вкладкам сразу выставляется низкий уровень памяти WebView2,
    /// активной — обычный. В остальных режимах скрытые вкладки не сжимаются до приостановки.
    fn apply_memory_levels(&self) {
        let active = self.tabs.active_id();
        let trim = self.settings.performance_mode.trim_hidden_immediately();
        for (id, view) in &self.views {
            webview_ops::set_low_memory(view, trim && Some(*id) != active);
        }
    }

    /// Создаёт WebView2 вкладки (адрес — уже в форме для загрузки), настраивает размер и события.
    fn spawn_view(&mut self, id: TabId, load_url: &str, visible: bool) -> Result<()> {
        let view = build_content(
            &mut self.ctx,
            &self.window,
            &self.proxy,
            &self.theme_css,
            &self.theme,
            id,
            load_url,
            &self.allow_file_nav,
            visible,
            &self.launch_args,
        )?;
        let _ = view.set_bounds(self.content_rect());
        #[cfg(windows)]
        {
            hook_source_changed(&view, id, self.proxy.clone());
            hook_accelerators(&view, self.proxy.clone());
            hook_fullscreen(&view, id, self.proxy.clone());
            hook_downloads(&view, id, self.proxy.clone());
        }
        let z = self.tabs.get(id).map(|t| t.zoom).unwrap_or(1.0);
        if (z - 1.0).abs() > 1e-6 {
            let _ = view.zoom(z);
        }
        self.views.insert(id, view);
        Ok(())
    }

    fn active_view(&self) -> Option<&WebView> {
        self.tabs.active_id().and_then(|id| self.views.get(&id))
    }

    /// Открывает вкладку с собственным WebView2. При ошибке вкладка снимается с учёта.
    fn open_tab(&mut self, url: &str, activate: bool) -> Result<TabId> {
        if self.tabs.len() >= MAX_TABS {
            return Err(anyhow!("достигнут предел в {MAX_TABS} вкладок"));
        }
        if url.to_ascii_lowercase().starts_with("file:") {
            self.allow_file_nav.store(true, Ordering::SeqCst);
        }
        let id = self
            .tabs
            .open(to_display_url(url), self.private, now_ms(), activate);
        let default_zoom = self.settings.default_zoom;
        if let Some(t) = self.tabs.get_mut(id) {
            t.loading = true;
            t.zoom = default_zoom;
        }
        match self.spawn_view(id, url, activate) {
            Ok(()) => {
                if activate {
                    self.show_only_active();
                    self.sync_window_title();
                }
                self.push_state();
                self.save_session();
                Ok(id)
            }
            Err(e) => {
                self.tabs.forget(id, now_ms());
                self.show_only_active();
                self.push_state();
                Err(e)
            }
        }
    }

    fn activate_tab(&mut self, id: TabId) {
        let prev = self.tabs.get(id).map(|t| t.state);
        if self.tabs.active_id() == Some(id) || !self.tabs.activate(id, now_ms()) {
            return;
        }
        match prev {
            // Выгруженная вкладка создаётся заново и загружает свой адрес.
            Some(TabState::Discarded) => self.restore_discarded(id),
            Some(TabState::Suspended) => {
                self.tabs.wake(id);
                if let Some(v) = self.views.get(&id) {
                    webview_ops::resume(v);
                }
            }
            _ => {}
        }
        self.show_only_active();
        self.sync_window_title();
        self.push_state();
        self.save_session();
    }

    /// Закрывает вкладку и уничтожает её WebView2 (освобождает процесс рендеринга и память).
    /// Возвращает `true`, если закрыта последняя вкладка и приложению пора завершиться.
    fn close_tab(&mut self, id: TabId) -> bool {
        self.tabs.close(id, now_ms());
        self.views.remove(&id);
        self.save_session();
        if self.tabs.is_empty() {
            self.shutdown();
            return true;
        }
        self.show_only_active();
        self.sync_window_title();
        self.push_state();
        false
    }

    fn reopen_closed_tab(&mut self) {
        let before = self.tabs.closed_count();
        if before == 0 {
            return;
        }
        let Some(id) = self.tabs.reopen_closed(now_ms()) else {
            return;
        };
        let url = self.tabs.get(id).map(|t| t.url.clone()).unwrap_or_default();
        let load = match parse_load_url(&url) {
            Some(page) => internal_load_url(page),
            None => url,
        };
        match self.spawn_view(id, &load, true) {
            Ok(()) => {
                if let Some(t) = self.tabs.get_mut(id) {
                    t.loading = true;
                }
            }
            Err(_) => {
                self.tabs.forget(id, now_ms());
            }
        }
        self.show_only_active();
        self.sync_window_title();
        self.push_state();
    }

    /// Возвращает `true`, если приложение должно завершиться.
    fn handle(&mut self, ev: UserEvent) -> bool {
        match ev {
            UserEvent::Ui(msg) => return self.command(msg),
            UserEvent::Page(id, UiMessage::Navigate { input }) => self.navigate(id, &input),
            UserEvent::Page(id, msg) => self.page_message(id, msg),
            UserEvent::NavStarted(id, url) => {
                if self.tabs.get(id).is_some() {
                    self.tabs.on_navigated(id, to_display_url(&url));
                    self.set_loading(id, true);
                    self.push_state();
                }
            }
            UserEvent::UrlChanged(id, url) => {
                if self.tabs.get(id).is_some() {
                    self.tabs.on_navigated(id, to_display_url(&url));
                    self.push_state();
                }
            }
            UserEvent::LoadStarted(id) => {
                self.set_loading(id, true);
                self.push_state();
            }
            UserEvent::LoadFinished(id) => {
                self.set_loading(id, false);
                self.record_visit(id);
                self.push_state();
                self.save_session();
            }
            UserEvent::Title(id, title) => {
                self.set_title(id, title);
                self.push_state();
            }
            UserEvent::NewWindow(url) => {
                if self.popup_throttle.allow(now_ms()) {
                    let _ = self.open_tab(&url, true);
                }
            }
            UserEvent::ChromeReady => self.push_state(),
            UserEvent::InputProbe(id, dirty, kind) => self.on_probe(id, dirty, kind),
            UserEvent::SuspendDone(id, ok) => self.on_suspend_done(id, ok),
            UserEvent::Cmd(cmd) => return self.run_command(cmd),
            UserEvent::HtmlFullscreen(id, on) => {
                // Только активная вкладка может раскрыть окно.
                if self.tabs.active_id() == Some(id) || !on {
                    self.html_fullscreen = on;
                    self.apply_fullscreen();
                }
            }
            UserEvent::Download(ev) => self.on_download(ev),
        }
        false
    }

    fn command(&mut self, msg: UiMessage) -> bool {
        match msg {
            UiMessage::Navigate { input } => {
                if let Some(id) = self.tabs.active_id() {
                    self.navigate(id, &input);
                }
            }
            UiMessage::Back => {
                if let Some(v) = self.active_view() {
                    let _ = v.go_back();
                }
            }
            UiMessage::Forward => {
                if let Some(v) = self.active_view() {
                    let _ = v.go_forward();
                }
            }
            UiMessage::Reload => {
                if let Some(v) = self.active_view() {
                    let _ = v.reload();
                }
            }
            UiMessage::Stop => {
                if let Some(v) = self.active_view() {
                    let _ = v.evaluate_script("window.stop()");
                }
                if let Some(id) = self.tabs.active_id() {
                    self.set_loading(id, false);
                }
                self.push_state();
            }
            UiMessage::NewTab => {
                let _ = self.open_tab(&internal_load_url(InternalPage::NewTab), true);
            }
            UiMessage::CloseTab { id } => return self.close_tab(TabId(id)),
            UiMessage::ActivateTab { id } => self.activate_tab(TabId(id)),
            UiMessage::MoveTab { id, index } => {
                if self.tabs.move_tab(TabId(id), index) {
                    self.push_state();
                }
            }
            UiMessage::ReopenClosedTab => self.reopen_closed_tab(),
            UiMessage::OpenInternal { page } => {
                if let Some(p) = InternalPage::from_host(&page) {
                    self.open_internal(p);
                }
            }
            m @ (UiMessage::PerfSnapshot
            | UiMessage::FreeMemory
            | UiMessage::SetMode { .. }
            | UiMessage::SetSleeping { .. }) => self.memory_message(self.tabs.active_id(), m),
            UiMessage::Key {
                ctrl,
                shift,
                alt,
                key,
            } => {
                if let Some(k) = shortcuts::key_from_name(&key) {
                    if let Some(cmd) = shortcuts::lookup(Chord::new(ctrl, shift, alt, k)) {
                        return self.run_command(cmd);
                    }
                }
            }
            UiMessage::ToggleBookmark => self.toggle_bookmark(),
            // Остальные сообщения (закладки, история, загрузки, настройки) приходят только от внутренних страниц.
            _ => {}
        }
        false
    }

    /// Единая точка исполнения команд: горячие клавиши панели, горячие клавиши страниц и внутренние страницы.
    /// Возвращает `true`, если приложение должно завершиться.
    fn run_command(&mut self, cmd: Command) -> bool {
        match cmd {
            Command::NewTab => {
                let _ = self.open_tab(&internal_load_url(InternalPage::NewTab), true);
                self.focus_address();
            }
            Command::NewPrivateTab => self.open_private_window(),
            Command::CloseActiveTab => {
                if let Some(id) = self.tabs.active_id() {
                    return self.close_tab(id);
                }
            }
            Command::ReopenClosedTab => self.reopen_closed_tab(),
            Command::NextTab => {
                if let Some(id) = self.tabs.next() {
                    self.activate_tab(id);
                }
            }
            Command::PrevTab => {
                if let Some(id) = self.tabs.prev() {
                    self.activate_tab(id);
                }
            }
            Command::SelectTab(i) => {
                // Ctrl+1..8: если вкладок меньше, чем номер, ничего не происходит (как в других браузерах).
                if let Some(id) = self.tabs.nth(i) {
                    self.activate_tab(id);
                }
            }
            Command::SelectLastTab => {
                if let Some(id) = self.tabs.last() {
                    self.activate_tab(id);
                }
            }
            Command::FocusAddressBar => self.focus_address(),
            Command::Back => {
                let _ = self.command(UiMessage::Back);
            }
            Command::Forward => {
                let _ = self.command(UiMessage::Forward);
            }
            // У WebView2 нет API «обновить в обход кэша» — Ctrl+Shift+R работает как обычное обновление.
            Command::Reload | Command::HardReload => {
                let _ = self.command(UiMessage::Reload);
            }
            Command::ZoomIn => self.zoom_active(Some(true)),
            Command::ZoomOut => self.zoom_active(Some(false)),
            Command::ZoomReset => self.zoom_active(None),
            Command::ToggleFullscreen => {
                self.user_fullscreen = !self.user_fullscreen;
                if !self.user_fullscreen {
                    self.html_fullscreen = false;
                }
                self.apply_fullscreen();
            }
            Command::BookmarkPage => self.toggle_bookmark(),
            Command::ShowHistory => self.open_internal(InternalPage::History),
            Command::ShowBookmarks => self.open_internal(InternalPage::Bookmarks),
            Command::ShowDownloads => self.open_internal(InternalPage::Downloads),
            Command::FreeMemory => {
                let _ = self.free_memory();
            }
        }
        false
    }

    fn focus_address(&self) {
        let _ = self.chrome.focus();
        let _ = self
            .chrome
            .evaluate_script("window.nexo&&window.nexo.focusAddress&&window.nexo.focusAddress();");
    }

    /// `Some(true)` — крупнее, `Some(false)` — мельче, `None` — вернуть к 100 %.
    fn zoom_active(&mut self, up: Option<bool>) {
        let Some(id) = self.tabs.active_id() else {
            return;
        };
        let cur = self.tabs.get(id).map(|t| t.zoom).unwrap_or(1.0);
        let next = match up {
            Some(up) => zoom::step(cur, up),
            None => 1.0,
        };
        if let Some(v) = self.views.get(&id) {
            if v.zoom(next).is_ok() {
                if let Some(t) = self.tabs.get_mut(id) {
                    t.zoom = next;
                }
            }
        }
    }

    /// Приватное окно — отдельный процесс с временным профилем: куки, кэш и хранилище не пересекаются с основным.
    fn open_private_window(&self) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).arg(PRIVATE_FLAG).spawn();
        }
    }

    // -----------------------------------------------------------------------------------------
    // Закладки, история, настройки, сессия
    // -----------------------------------------------------------------------------------------

    fn can_bookmark(&self, url: &str) -> bool {
        let l = url.to_ascii_lowercase();
        !self.private
            && self.db.is_some()
            && (l.starts_with("http://") || l.starts_with("https://"))
    }

    fn is_bookmarked(&mut self, url: &str) -> bool {
        if let Some((u, b)) = &self.bookmark_cache {
            if u == url {
                return *b;
            }
        }
        let b = self
            .db
            .as_ref()
            .and_then(|d| d.find_bookmark_by_url(url).ok().flatten())
            .is_some();
        self.bookmark_cache = Some((url.to_string(), b));
        b
    }

    fn toggle_bookmark(&mut self) {
        let Some(tab) = self.tabs.active() else {
            return;
        };
        let (url, title) = (tab.url.clone(), tab.title.clone());
        if !self.can_bookmark(&url) {
            return;
        }
        if let Some(db) = &self.db {
            match db.find_bookmark_by_url(&url) {
                Ok(Some(b)) => {
                    let _ = db.remove_bookmark(b.id);
                }
                Ok(None) => {
                    let name = if title.is_empty() { url.clone() } else { title };
                    let _ = db.add_bookmark(None, &name, &url, now_ms());
                }
                Err(_) => {}
            }
        }
        self.bookmark_cache = None;
        self.push_state();
        self.refresh_page(InternalPage::Bookmarks);
    }

    fn save_session(&self) {
        if self.private {
            return;
        }
        let Some(db) = &self.db else { return };
        let active = self.tabs.active_id();
        let tabs: Vec<SessionTab> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| !t.url.is_empty())
            .map(|t| SessionTab {
                url: t.url.clone(),
                title: t.title.clone(),
                pinned: t.pinned,
                zoom: t.zoom,
                active: Some(t.id) == active,
            })
            .collect();
        let _ = db.save_session(&tabs);
    }

    /// Восстанавливает вкладки прошлого сеанса. Загружается только активная; остальные остаются
    /// выгруженными (адрес и заголовок известны) и создают WebView2 при первом открытии —
    /// быстрый запуск и минимум памяти. Возвращает `false`, если восстанавливать нечего.
    fn restore_session(&mut self) -> bool {
        let Some(saved) = self.db.as_ref().and_then(|d| d.load_session().ok()) else {
            return false;
        };
        let saved: Vec<SessionTab> = saved.into_iter().take(MAX_TABS).collect();
        if saved.is_empty() {
            return false;
        }
        let mut active_id = None;
        for st in &saved {
            let id = self.tabs.open(st.url.clone(), false, now_ms(), false);
            self.tabs.set_state(id, TabState::Discarded);
            self.tabs.set_title(id, st.title.clone());
            if let Some(t) = self.tabs.get_mut(id) {
                t.zoom = if st.zoom.is_finite() {
                    st.zoom.clamp(0.25, 5.0)
                } else {
                    1.0
                };
            }
            if st.pinned {
                self.tabs.set_pinned(id, true);
            }
            if st.active || active_id.is_none() {
                active_id = Some(id);
            }
        }
        let Some(active) = active_id else {
            return false;
        };
        self.tabs.activate(active, now_ms());
        self.restore_discarded(active);
        if self.views.is_empty() {
            return false;
        }
        self.show_only_active();
        self.sync_window_title();
        self.push_state();
        true
    }

    /// Закрытие окна: сохраняем сессию; временный профиль приватного окна удаляется.
    fn shutdown(&mut self) {
        self.save_session();
        if let Some(dir) = self.private_dir.take() {
            // Сначала освобождаем WebView2, чтобы процессы отпустили файлы профиля.
            self.views.clear();
            schedule_dir_removal(&dir);
        }
    }

    fn page_message(&mut self, id: TabId, msg: UiMessage) {
        match msg {
            UiMessage::Navigate { input } => self.navigate(id, &input),
            UiMessage::PageData => self.send_page_data(id),
            UiMessage::BookmarkRemove { id: b } => {
                if let Some(db) = &self.db {
                    let _ = db.remove_bookmark(b);
                }
                self.bookmark_cache = None;
                self.push_state();
                self.refresh_page(InternalPage::Bookmarks);
            }
            UiMessage::BookmarkMove { id: b, folder } => {
                if let Some(db) = &self.db {
                    let _ = db.move_bookmark(b, folder);
                }
                self.refresh_page(InternalPage::Bookmarks);
            }
            UiMessage::FolderAdd { name } => {
                let name = name.trim();
                if !name.is_empty() && name.chars().count() <= 100 {
                    if let Some(db) = &self.db {
                        let _ = db.add_folder(None, name);
                    }
                }
                self.refresh_page(InternalPage::Bookmarks);
            }
            UiMessage::FolderRemove { id: f } => {
                if let Some(db) = &self.db {
                    let _ = db.remove_folder(f);
                }
                self.bookmark_cache = None;
                self.push_state();
                self.refresh_page(InternalPage::Bookmarks);
            }
            UiMessage::HistorySearch { query } => {
                self.history_query = query.chars().take(200).collect();
                self.refresh_page(InternalPage::History);
            }
            UiMessage::HistoryDelete { id: h } => {
                if let Some(db) = &self.db {
                    let _ = db.delete_history_entry(h);
                }
                self.refresh_page(InternalPage::History);
            }
            UiMessage::HistoryClear => {
                if let Some(db) = &self.db {
                    let _ = db.clear_history();
                }
                self.refresh_page(InternalPage::History);
            }
            UiMessage::DownloadCancel { id: d } => {
                cancel_download(d);
                self.downloads.cancel(d);
                self.refresh_page(InternalPage::Downloads);
            }
            UiMessage::DownloadShow { id: d } => {
                if let Some(path) = self.downloads.get(d).map(|i| i.path.clone()) {
                    show_in_folder(&path);
                }
            }
            UiMessage::DownloadsClearFinished => {
                self.downloads.clear_finished();
                self.refresh_page(InternalPage::Downloads);
            }
            UiMessage::UpdateSettings {
                theme,
                search_engine,
                restore_session,
                homepage,
                default_zoom_pct,
            } => {
                if let Some(t) = theme {
                    self.settings.theme = t;
                }
                if let Some(e) = search_engine {
                    self.settings.search_engine = e;
                }
                if let Some(r) = restore_session {
                    self.settings.restore_session = r;
                }
                if let Some(h) = homepage {
                    self.settings.homepage = h.trim().chars().take(2048).collect();
                }
                if let Some(z) = default_zoom_pct {
                    self.settings.default_zoom = f64::from(z) / 100.0;
                }
                self.settings = std::mem::take(&mut self.settings).sanitized();
                if !self.private {
                    self.save_settings();
                }
                self.refresh_page(InternalPage::Settings);
            }
            other => self.memory_message(Some(id), other),
        }
    }

    /// Отправляет данные странице вкладки `id` — по адресу вкладки видно, какая это страница.
    fn send_page_data(&mut self, id: TabId) {
        let Some(page) = self.tabs.get(id).and_then(|t| parse_load_url(&t.url)) else {
            return;
        };
        let json = match page {
            InternalPage::Bookmarks => self.db.as_ref().map(data::bookmarks_json),
            InternalPage::History => self
                .db
                .as_ref()
                .map(|d| data::history_json(d, &self.history_query)),
            InternalPage::Downloads => Some(data::downloads_json(&self.downloads)),
            InternalPage::Settings => Some(data::settings_json(&self.settings)),
            _ => None,
        };
        let json = json.unwrap_or_else(|| serde_json::json!({}));
        if let Some(view) = self.views.get(&id) {
            let _ = view.evaluate_script(&data::to_script(page.host(), &json));
        }
    }

    /// Обновляет все открытые вкладки с этой внутренней страницей.
    fn refresh_page(&mut self, page: InternalPage) {
        let display = page.url();
        let ids: Vec<TabId> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| t.url == display && t.is_live())
            .map(|t| t.id)
            .collect();
        for id in ids {
            self.send_page_data(id);
        }
    }

    fn on_download(&mut self, ev: DownloadEvent) {
        match ev {
            DownloadEvent::Started {
                tab,
                id,
                url,
                path,
                total,
            } => {
                self.downloads.start_with_id(id, url, path, total, now_ms());
                // Вкладку с идущей загрузкой не усыпляем.
                if let Some(t) = self.tabs.get_mut(tab) {
                    t.downloading = true;
                }
                self.refresh_page(InternalPage::Downloads);
            }
            DownloadEvent::Progress {
                id,
                received,
                total,
            } => {
                self.downloads.update(id, received, total);
                if self.dl_throttle.allow(now_ms()) {
                    self.refresh_page(InternalPage::Downloads);
                }
            }
            DownloadEvent::Ended { id, ok } => {
                if ok {
                    self.downloads.complete(id);
                } else {
                    self.downloads.fail(id);
                }
                if !self.downloads.has_active() {
                    for t in self.tabs.tabs().to_vec() {
                        if t.downloading {
                            if let Some(m) = self.tabs.get_mut(t.id) {
                                m.downloading = false;
                            }
                        }
                    }
                }
                self.refresh_page(InternalPage::Downloads);
            }
        }
    }

    /// Открывает внутреннюю страницу; если она уже открыта — переключается на неё.
    fn open_internal(&mut self, page: InternalPage) {
        let display = page.url();
        let existing = self
            .tabs
            .tabs()
            .iter()
            .find(|t| t.url == display)
            .map(|t| t.id);
        match existing {
            Some(id) => self.activate_tab(id),
            None => {
                let _ = self.open_tab(&internal_load_url(page), true);
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Управление памятью
    // -----------------------------------------------------------------------------------------

    fn wait_mode(&self) -> ControlFlow {
        if self.settings.sleeping_enabled {
            ControlFlow::WaitUntil(self.next_tick)
        } else {
            ControlFlow::Wait
        }
    }

    fn schedule_tick(&mut self) {
        self.next_tick =
            Instant::now() + Duration::from_secs(self.settings.performance_mode.tick_secs());
    }

    /// Обновляет признак «играет звук» у фоновых живых вкладок (официальное свойство WebView2).
    fn refresh_audio(&mut self) {
        let active = self.tabs.active_id();
        let ids: Vec<TabId> = self
            .tabs
            .tabs()
            .iter()
            .filter(|t| t.state == TabState::Active && Some(t.id) != active)
            .map(|t| t.id)
            .collect();
        for id in ids {
            let playing = self
                .views
                .get(&id)
                .map(webview_ops::is_playing_audio)
                .unwrap_or(false);
            if let Some(t) = self.tabs.get_mut(id) {
                t.playing_audio = playing;
            }
        }
    }

    fn tick(&mut self) {
        if !self.settings.sleeping_enabled {
            return;
        }
        self.refresh_audio();
        let policy = self.settings.performance_mode.policy();
        let actions = sleeper::plan(self.tabs.tabs(), self.tabs.active_id(), &policy, now_ms());
        self.execute_sleep(actions);
    }

    /// Запускает план сна. Перед приостановкой/выгрузкой живой вкладки спрашиваем страницу,
    /// не вводил ли пользователь данные; ответ приходит событием `InputProbe`.
    /// Возвращает, сколько вкладок поставлено в работу.
    fn execute_sleep(&mut self, actions: Vec<SleepAction>) -> usize {
        let mut started = 0;
        for action in actions {
            let (id, kind) = match action {
                SleepAction::Suspend(id) => (id, SleepKind::Suspend),
                SleepAction::Discard(id) => (id, SleepKind::Discard),
            };
            if !self.sleep_pending.insert(id) {
                continue;
            }
            let already_suspended = self
                .tabs
                .get(id)
                .map(|t| t.state == TabState::Suspended)
                .unwrap_or(false);
            if already_suspended {
                // Приостановленная страница не выполняет JS, но её состояние не меняется
                // с момента проверки перед приостановкой — повторная проверка не нужна.
                self.apply_sleep(id, kind);
                started += 1;
                continue;
            }
            let proxy = self.proxy.clone();
            let sent = self
                .views
                .get(&id)
                .map(|v| {
                    v.evaluate_script_with_callback(PROBE_JS, move |r| {
                        let _ =
                            proxy.send_event(UserEvent::InputProbe(id, r.trim() == "true", kind));
                    })
                    .is_ok()
                })
                .unwrap_or(false);
            if sent {
                started += 1;
            } else {
                self.sleep_pending.remove(&id);
            }
        }
        started
    }

    fn on_probe(&mut self, id: TabId, dirty: bool, kind: SleepKind) {
        if dirty {
            if let Some(t) = self.tabs.get_mut(id) {
                t.has_unsaved_input = true;
            }
            self.sleep_pending.remove(&id);
            return;
        }
        // Пока шла проверка, вкладка могла стать активной или получить защиту.
        let ok = self.tabs.active_id() != Some(id)
            && self
                .tabs
                .get(id)
                .map(|t| t.is_live() && !t.is_protected())
                .unwrap_or(false);
        if ok {
            self.apply_sleep(id, kind);
        } else {
            self.sleep_pending.remove(&id);
        }
    }

    fn apply_sleep(&mut self, id: TabId, kind: SleepKind) {
        match kind {
            SleepKind::Discard => {
                self.sleep_pending.remove(&id);
                if self.tabs.mark_discarded(id) {
                    // Освобождается WebView2 вкладки (и её процесс рендеринга, если он больше никому не нужен).
                    self.views.remove(&id);
                    self.push_state();
                }
            }
            SleepKind::Suspend => {
                let proxy = self.proxy.clone();
                let started = self
                    .views
                    .get(&id)
                    .map(|v| {
                        let _ = v.set_visible(false); // TrySuspend требует скрытого WebView2
                        webview_ops::suspend(v, move |ok| {
                            let _ = proxy.send_event(UserEvent::SuspendDone(id, ok));
                        })
                    })
                    .unwrap_or(false);
                if !started {
                    self.sleep_pending.remove(&id);
                }
            }
        }
    }

    fn on_suspend_done(&mut self, id: TabId, ok: bool) {
        self.sleep_pending.remove(&id);
        if !ok {
            return;
        }
        if self.tabs.mark_suspended(id) {
            self.push_state();
        } else if let Some(v) = self.views.get(&id) {
            // Пока шла приостановка, вкладка стала активной или защищённой — возвращаем её к жизни.
            webview_ops::resume(v);
        }
    }

    /// Пересоздаёт WebView2 выгруженной вкладки. При ошибке вкладка остаётся выгруженной.
    fn restore_discarded(&mut self, id: TabId) {
        let Some(url) = self.tabs.get(id).map(|t| t.url.clone()) else {
            return;
        };
        let load = match parse_load_url(&url) {
            Some(page) => internal_load_url(page),
            None => url,
        };
        if load.to_ascii_lowercase().starts_with("file:") {
            self.allow_file_nav.store(true, Ordering::SeqCst);
        }
        match self.spawn_view(id, &load, true) {
            Ok(()) => {
                self.tabs.wake(id);
                if let Some(t) = self.tabs.get_mut(id) {
                    t.loading = true;
                    t.has_unsaved_input = false;
                }
            }
            Err(_) => self.tabs.set_state(id, TabState::Discarded),
        }
    }

    /// Кнопка «Освободить память»: выгружает всё, что разрешено правилами защиты.
    fn free_memory(&mut self) -> String {
        self.refresh_audio();
        let protected = self.tabs.protected_background_count();
        let actions = sleeper::plan_free_memory(self.tabs.tabs(), self.tabs.active_id(), now_ms());
        let started = self.execute_sleep(actions);
        free_memory_notice(started, protected)
    }

    fn save_settings(&self) {
        if let Some(db) = &self.db {
            let _ = db.save_settings(&self.settings);
        }
    }

    fn memory_message(&mut self, target: Option<TabId>, msg: UiMessage) {
        let mut notice = None;
        match msg {
            UiMessage::PerfSnapshot => {}
            UiMessage::FreeMemory => notice = Some(self.free_memory()),
            UiMessage::SetMode { mode } => {
                self.settings.performance_mode = mode;
                self.save_settings();
                self.schedule_tick();
                self.apply_memory_levels();
            }
            UiMessage::SetSleeping { enabled } => {
                self.settings.sleeping_enabled = enabled;
                self.save_settings();
                self.schedule_tick();
            }
            _ => return,
        }
        let Some(id) = target else { return };
        if let Some(text) = notice {
            if let (Some(view), Ok(json)) = (self.views.get(&id), serde_json::to_string(&text)) {
                let _ = view
                    .evaluate_script(&format!("window.nexoPerf&&window.nexoPerf.notice({json});"));
            }
        }
        self.send_perf(id);
    }

    /// Снимает реальные метрики процессов и отправляет их странице производительности.
    fn send_perf(&mut self, id: TabId) {
        let sample = monitor::sample_tree().map_err(|e| e.to_string());
        let cpu = sample.as_ref().ok().and_then(|s| self.cpu.update(s));
        let mode: PerformanceMode = self.settings.performance_mode;
        let report = build_report(ReportInput {
            sample: sample.as_ref().map_err(|e| e.clone()),
            cpu_percent: cpu,
            tabs: self.tabs.tabs(),
            active: self.tabs.active_id(),
            mode,
            sleeping_enabled: self.settings.sleeping_enabled,
            restart_needed: mode.browser_args() != self.launch_args,
            now_ms: now_ms(),
        });
        if let Some(view) = self.views.get(&id) {
            let _ = view.evaluate_script(&report.to_script());
        }
    }

    fn navigate(&mut self, id: TabId, input: &str) {
        let target = resolve_input(input, self.settings.search_engine);
        let url = target.load_url();
        if url.to_ascii_lowercase().starts_with("file:") {
            self.allow_file_nav.store(true, Ordering::SeqCst);
        }
        let Some(loaded) = self.views.get(&id).map(|v| v.load_url(&url).is_ok()) else {
            return;
        };
        if loaded {
            self.tabs.on_navigated(id, to_display_url(&url));
            self.set_loading(id, true);
            self.push_state();
        }
        // Фокус — странице, чтобы можно было сразу прокручивать и печатать.
        if self.tabs.active_id() == Some(id) {
            if let Some(view) = self.views.get(&id) {
                let _ = view.focus();
            }
        }
    }

    fn set_loading(&mut self, id: TabId, loading: bool) {
        if let Some(t) = self.tabs.get_mut(id) {
            t.loading = loading;
        }
    }

    fn set_title(&mut self, id: TabId, title: String) {
        let Some(url) = self.tabs.get(id).map(|t| t.url.clone()) else {
            return;
        };
        if let Some(db) = &self.db {
            let _ = db.update_history_title(&url, &title);
        }
        self.tabs.set_title(id, title);
        if self.tabs.active_id() == Some(id) {
            self.sync_window_title();
        }
    }

    fn sync_window_title(&self) {
        let title = self.tabs.active().map(|t| t.title.as_str()).unwrap_or("");
        let app = if self.private {
            "Nexo Browser — приватное окно"
        } else {
            "Nexo Browser"
        };
        let full = if title.is_empty() {
            app.to_string()
        } else {
            format!("{title} — {app}")
        };
        self.window.set_title(&full);
    }

    fn record_visit(&self, id: TabId) {
        let (Some(db), Some(tab)) = (&self.db, self.tabs.get(id)) else {
            return;
        };
        // Внутренние страницы и приватные вкладки в историю не попадают (фильтр — в record_visit).
        if !tab.private {
            let _ = db.record_visit(&tab.url, &tab.title, now_ms());
        }
    }

    fn push_state(&mut self) {
        let active = self.tabs.active();
        let url = active.map(|t| t.url.clone()).unwrap_or_default();
        let can_bookmark = self.can_bookmark(&url);
        let bookmarked = can_bookmark && self.is_bookmarked(&url);
        let view = self.active_view();
        let state = UiState {
            tabs: self
                .tabs
                .tabs()
                .iter()
                .map(|t| TabView {
                    id: t.id.0,
                    title: tab_label(t),
                    active: Some(t.id) == self.tabs.active_id(),
                    loading: t.loading,
                    sleeping: t.state != TabState::Active,
                })
                .collect(),
            address: address_text(&url),
            security: security_of(&url),
            can_back: view.and_then(|v| v.can_go_back().ok()).unwrap_or(false),
            can_forward: view.and_then(|v| v.can_go_forward().ok()).unwrap_or(false),
            loading: self.tabs.active().map(|t| t.loading).unwrap_or(false),
            bookmarked,
            can_bookmark,
        };
        let _ = self.chrome.evaluate_script(&state.to_script());
    }
}

fn tab_label(t: &TabInfo) -> String {
    if !t.title.is_empty() {
        t.title.clone()
    } else if t.url == InternalPage::NewTab.url() || t.url.is_empty() {
        "Новая вкладка".to_string()
    } else {
        t.url.clone()
    }
}

fn startup_target(settings: &Settings) -> Target {
    // Адрес из командной строки (`nexo.exe https://example.com`) важнее домашней страницы.
    match cli_url() {
        Some(a) => resolve_input(&a, settings.search_engine),
        None if !settings.homepage.trim().is_empty() => {
            resolve_input(&settings.homepage, settings.search_engine)
        }
        None => Target::Internal(InternalPage::NewTab),
    }
}

/// Адрес из командной строки: первый аргумент без `--` (флаги вроде `--private` — не адреса).
fn cli_url() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--") && !a.trim().is_empty())
}

fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Nexo")
}

fn pick_theme(choice: ThemeChoice, window: &Window) -> (&'static Theme, bool) {
    let dark = match choice {
        ThemeChoice::Dark => true,
        ThemeChoice::Light => false,
        ThemeChoice::System => system_is_dark(window),
    };
    (if dark { &theme::DARK } else { &theme::LIGHT }, dark)
}

// ---------------------------------------------------------------------------------------------
// Windows-специфика
// ---------------------------------------------------------------------------------------------

#[cfg(windows)]
fn system_is_dark(window: &Window) -> bool {
    use tao::platform::windows::WindowExtWindows;
    matches!(window.theme(), tao::window::Theme::Dark)
}

#[cfg(not(windows))]
fn system_is_dark(_window: &Window) -> bool {
    true
}

/// Тёмный заголовок окна в цвет градиента (Windows 10 2004+ — тёмный режим; Windows 11 — ещё и цвет).
#[cfg(windows)]
fn style_title_bar(window: &Window, th: &Theme, dark: bool) {
    use std::ffi::c_void;
    use tao::platform::windows::WindowExtWindows;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    };

    let hwnd = HWND(window.hwnd() as *mut c_void);
    let colorref = |c: theme::Rgb| (c.0 as u32) | ((c.1 as u32) << 8) | ((c.2 as u32) << 16);
    let dark_flag: i32 = dark as i32;
    let caption = colorref(th.bg_start);
    let text = colorref(th.text);
    // Ошибки игнорируем: на старых версиях Windows эти атрибуты просто не поддерживаются.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_flag as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(not(windows))]
fn style_title_bar(_window: &Window, _th: &Theme, _dark: bool) {}

/// Событие WebView2 `SourceChanged` срабатывает и при смене URL без загрузки страницы
/// (YouTube, GitHub, Reddit и другие SPA) — без него адресная строка показывала бы устаревший адрес.
#[cfg(windows)]
fn hook_source_changed(view: &WebView, id: TabId, proxy: EventLoopProxy<UserEvent>) {
    use webview2_com::{take_pwstr, SourceChangedEventHandler};
    use windows::core::PWSTR;
    use wry::WebViewExtWindows;

    let webview = view.webview();
    let mut token = 0i64;
    let handler = SourceChangedEventHandler::create(Box::new(move |sender, _args| {
        if let Some(sender) = sender {
            let mut uri = PWSTR::null();
            unsafe { sender.Source(&mut uri)? };
            let url = take_pwstr(uri);
            let _ = proxy.send_event(UserEvent::UrlChanged(id, url));
        }
        Ok(())
    }));
    unsafe {
        let _ = webview.add_SourceChanged(&handler, &mut token);
    }
}

/// Горячие клавиши, нажатые в странице сайта. Фокус в WebView2 страницы не доходит до окна, поэтому
/// без этого обработчика Ctrl+T/W/L/R и другие работали бы только при фокусе на панели интерфейса.
/// Клавиша перехватывается (и не доходит до сайта), только если она есть в таблице `shortcuts`.
#[cfg(windows)]
fn hook_accelerators(view: &WebView, proxy: EventLoopProxy<UserEvent>) {
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
        COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
    use wry::WebViewExtWindows;

    let controller = view.controller();
    let mut token = 0i64;
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_sender, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND(0);
        unsafe { args.KeyEventKind(&mut kind)? };
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
            && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
        {
            return Ok(());
        }
        let mut vk = 0u32;
        unsafe { args.VirtualKey(&mut vk)? };
        let Some(key) = shortcuts::key_from_vk(vk) else {
            return Ok(());
        };
        let down = |k: i32| unsafe { GetKeyState(k) } < 0;
        let chord = Chord::new(
            down(VK_CONTROL.0 as i32),
            down(VK_SHIFT.0 as i32),
            down(VK_MENU.0 as i32),
            key,
        );
        if let Some(cmd) = shortcuts::lookup(chord) {
            unsafe { args.SetHandled(true)? };
            let _ = proxy.send_event(UserEvent::Cmd(cmd));
        }
        Ok(())
    }));
    unsafe {
        let _ = controller.add_AcceleratorKeyPressed(&handler, &mut token);
    }
}

/// Полноэкранный режим из страницы (кнопка на видео): окно раскрывается, панель прячется.
#[cfg(windows)]
fn hook_fullscreen(view: &WebView, id: TabId, proxy: EventLoopProxy<UserEvent>) {
    use webview2_com::ContainsFullScreenElementChangedEventHandler;
    use windows::core::BOOL;
    use wry::WebViewExtWindows;

    let webview = view.webview();
    let mut token = 0i64;
    let handler =
        ContainsFullScreenElementChangedEventHandler::create(Box::new(move |sender, _args| {
            if let Some(sender) = sender {
                let mut on = BOOL::default();
                unsafe { sender.ContainsFullScreenElement(&mut on)? };
                let _ = proxy.send_event(UserEvent::HtmlFullscreen(id, on.as_bool()));
            }
            Ok(())
        }));
    unsafe {
        let _ = webview.add_ContainsFullScreenElementChanged(&handler, &mut token);
    }
}

#[cfg(windows)]
thread_local! {
    /// Идущие загрузки: нужны, чтобы отменить загрузку по номеру. Обработчики WebView2 и
    /// приложение работают в одном (главном) потоке.
    static DOWNLOAD_OPS: std::cell::RefCell<HashMap<u64, webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2DownloadOperation>> =
        std::cell::RefCell::new(HashMap::new());
}

#[cfg(windows)]
static NEXT_DOWNLOAD_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Загрузки через WebView2: свой список с реальным прогрессом (`BytesReceived`/`TotalBytesToReceive`).
/// Системная панель загрузок подавляется (`Handled`), файл сохраняется в папку, выбранную WebView2
/// (папка «Загрузки» пользователя).
#[cfg(windows)]
fn hook_downloads(view: &WebView, tab: TabId, proxy: EventLoopProxy<UserEvent>) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_4, COREWEBVIEW2_DOWNLOAD_STATE, COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED,
        COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED,
    };
    use webview2_com::{
        take_pwstr, BytesReceivedChangedEventHandler, DownloadStartingEventHandler,
        StateChangedEventHandler,
    };
    use windows::core::{Interface, PWSTR};
    use wry::WebViewExtWindows;

    let Ok(w4) = view.webview().cast::<ICoreWebView2_4>() else {
        return;
    };
    let mut token = 0i64;
    let handler = DownloadStartingEventHandler::create(Box::new(move |_sender, args| {
        let Some(args) = args else { return Ok(()) };
        let op = unsafe { args.DownloadOperation()? };
        let id = NEXT_DOWNLOAD_ID.fetch_add(1, Ordering::Relaxed);

        let mut uri = PWSTR::null();
        unsafe { op.Uri(&mut uri)? };
        let url = take_pwstr(uri);
        let mut path = PWSTR::null();
        unsafe { args.ResultFilePath(&mut path)? };
        let path = take_pwstr(path);
        let mut total = 0i64;
        unsafe { op.TotalBytesToReceive(&mut total)? };
        let total = (total > 0).then_some(total as u64);

        // Свой список загрузок вместо системной панели.
        unsafe { args.SetHandled(true)? };
        DOWNLOAD_OPS.with(|m| m.borrow_mut().insert(id, op.clone()));
        let _ = proxy.send_event(UserEvent::Download(DownloadEvent::Started {
            tab,
            id,
            url,
            path,
            total,
        }));

        let p = proxy.clone();
        let progress = BytesReceivedChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(op) = sender {
                let (mut got, mut all) = (0i64, 0i64);
                unsafe {
                    op.BytesReceived(&mut got)?;
                    op.TotalBytesToReceive(&mut all)?;
                }
                let _ = p.send_event(UserEvent::Download(DownloadEvent::Progress {
                    id,
                    received: got.max(0) as u64,
                    total: (all > 0).then_some(all as u64),
                }));
            }
            Ok(())
        }));
        let p = proxy.clone();
        let state = StateChangedEventHandler::create(Box::new(move |sender, _| {
            if let Some(op) = sender {
                let mut st = COREWEBVIEW2_DOWNLOAD_STATE::default();
                unsafe { op.State(&mut st)? };
                if st == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED
                    || st == COREWEBVIEW2_DOWNLOAD_STATE_INTERRUPTED
                {
                    DOWNLOAD_OPS.with(|m| m.borrow_mut().remove(&id));
                    let _ = p.send_event(UserEvent::Download(DownloadEvent::Ended {
                        id,
                        ok: st == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED,
                    }));
                }
            }
            Ok(())
        }));
        let mut t = 0i64;
        unsafe {
            let _ = op.add_BytesReceivedChanged(&progress, &mut t);
            let _ = op.add_StateChanged(&state, &mut t);
        }
        Ok(())
    }));
    unsafe {
        let _ = w4.add_DownloadStarting(&handler, &mut token);
    }
}

/// Отмена идущей загрузки. На не-Windows загрузок нет.
#[cfg(windows)]
fn cancel_download(id: u64) {
    DOWNLOAD_OPS.with(|m| {
        if let Some(op) = m.borrow().get(&id) {
            let _ = unsafe { op.Cancel() };
        }
    });
}

#[cfg(not(windows))]
fn cancel_download(_id: u64) {}

/// «Показать в папке»: открывает Проводник с выделенным файлом. Сам файл не запускается.
#[cfg(windows)]
fn show_in_folder(path: &str) {
    use std::os::windows::process::CommandExt;
    if path.contains('"') {
        return;
    }
    let _ = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{path}\""))
        .spawn();
}

#[cfg(not(windows))]
fn show_in_folder(_path: &str) {}

/// Удаляет временный профиль приватного окна. На Windows процессы WebView2 ещё могут держать файлы
/// в момент выхода, поэтому удаление делает отдельный короткий процесс после небольшой паузы.
#[cfg(windows)]
fn schedule_dir_removal(dir: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let d = dir.display().to_string();
    if d.contains('"') {
        return;
    }
    let _ = std::process::Command::new("cmd")
        .raw_arg(format!("/C ping -n 4 127.0.0.1 >nul & rmdir /s /q \"{d}\""))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(not(windows))]
fn schedule_dir_removal(dir: &std::path::Path) {
    let _ = std::fs::remove_dir_all(dir);
}

/// Сообщение об ошибке запуска (у программы нет консоли, поэтому без диалога сбой был бы невидим).
#[cfg(windows)]
pub fn show_fatal(err: &anyhow::Error) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let text = HSTRING::from(format!(
        "Не удалось запустить Nexo Browser.\n\n{err:#}\n\nУбедитесь, что установлен Microsoft Edge WebView2 Runtime."
    ));
    unsafe {
        MessageBoxW(
            None,
            &text,
            &HSTRING::from("Nexo Browser"),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
pub fn show_fatal(err: &anyhow::Error) {
    eprintln!("Не удалось запустить Nexo Browser: {err:#}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(v: bool) -> AtomicBool {
        AtomicBool::new(v)
    }

    #[test]
    fn web_schemes_allowed_dangerous_blocked() {
        let f = flag(false);
        assert!(navigation_allowed("https://example.com/", &f));
        assert!(navigation_allowed("http://example.com/", &f));
        assert!(navigation_allowed("about:blank", &f));
        assert!(!navigation_allowed("javascript:alert(1)", &f));
        assert!(!navigation_allowed("data:text/html,x", &f));
        assert!(!navigation_allowed("mailto:a@b.c", &f));
        assert!(!navigation_allowed("ms-settings:", &f));
    }

    #[test]
    fn file_navigation_needs_explicit_permission_once() {
        let f = flag(false);
        assert!(!navigation_allowed("file:///C:/a.html", &f));
        f.store(true, Ordering::SeqCst);
        assert!(navigation_allowed("file:///C:/a.html", &f));
        assert!(!navigation_allowed("file:///C:/a.html", &f)); // разрешение одноразовое
    }

    #[test]
    fn internal_pages_are_navigable() {
        let f = flag(false);
        assert!(navigation_allowed(
            &internal_load_url(InternalPage::NewTab),
            &f
        ));
        assert!(navigation_allowed(
            &internal_load_url(InternalPage::Performance),
            &f
        ));
    }

    #[test]
    fn tab_labels() {
        let mut t = TabInfo::new(TabId(1), "nexo://newtab".into(), false, 0);
        assert_eq!(tab_label(&t), "Новая вкладка");
        t.url = "https://a.com/".into();
        assert_eq!(tab_label(&t), "https://a.com/");
        t.title = "Site".into();
        assert_eq!(tab_label(&t), "Site");
    }
}
