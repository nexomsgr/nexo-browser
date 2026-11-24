//! Встроенные в .exe ресурсы интерфейса: ничего не читается с диска и не качается из сети.

const THEME_PLACEHOLDER: &str = "/*THEME*/";
const PAGECSS_PLACEHOLDER: &str = "/*PAGECSS*/";

const UI_HTML: &str = include_str!("../../assets/ui/index.html");
const UI_CSS: &str = include_str!("../../assets/ui/ui.css");
const UI_JS: &str = include_str!("../../assets/ui/ui.js");

const NEWTAB_HTML: &str = include_str!("../../assets/pages/newtab.html");
const NEWTAB_JS: &str = include_str!("../../assets/pages/newtab.js");
const PERF_HTML: &str = include_str!("../../assets/pages/performance.html");
const PERF_JS: &str = include_str!("../../assets/pages/performance.js");
const PAGE_CSS: &str = include_str!("../../assets/pages/page.css");
const PAGE_JS: &str = include_str!("../../assets/pages/page.js");
const BOOKMARKS_HTML: &str = include_str!("../../assets/pages/bookmarks.html");
const BOOKMARKS_JS: &str = include_str!("../../assets/pages/bookmarks.js");
const HISTORY_HTML: &str = include_str!("../../assets/pages/history.html");
const HISTORY_JS: &str = include_str!("../../assets/pages/history.js");
const DOWNLOADS_HTML: &str = include_str!("../../assets/pages/downloads.html");
const DOWNLOADS_JS: &str = include_str!("../../assets/pages/downloads.js");
const SETTINGS_HTML: &str = include_str!("../../assets/pages/settings.html");
const SETTINGS_JS: &str = include_str!("../../assets/pages/settings.js");

pub struct Asset {
    pub mime: &'static str,
    pub body: Vec<u8>,
}

fn html(src: &str, theme_css: &str) -> Asset {
    Asset {
        mime: "text/html; charset=utf-8",
        body: src
            .replace(THEME_PLACEHOLDER, theme_css)
            .replace(PAGECSS_PLACEHOLDER, PAGE_CSS)
            .into_bytes(),
    }
}

fn text(mime: &'static str, src: &str) -> Asset {
    Asset {
        mime,
        body: src.as_bytes().to_vec(),
    }
}

/// Ресурсы собственного интерфейса (протокол `nexo-ui`).
pub fn chrome_asset(path: &str, theme_css: &str) -> Option<Asset> {
    match path {
        "/" | "/index.html" => Some(html(UI_HTML, theme_css)),
        "/ui.css" => Some(text("text/css; charset=utf-8", UI_CSS)),
        "/ui.js" => Some(text("text/javascript; charset=utf-8", UI_JS)),
        _ => None,
    }
}

/// Внутренние страницы (протокол `nexo`): `nexo://newtab` и др.
pub fn page_asset(path: &str, theme_css: &str) -> Option<Asset> {
    match path {
        "/" | "/newtab" => Some(html(NEWTAB_HTML, theme_css)),
        "/newtab.js" => Some(text("text/javascript; charset=utf-8", NEWTAB_JS)),
        "/performance" => Some(html(PERF_HTML, theme_css)),
        "/performance.js" => Some(text("text/javascript; charset=utf-8", PERF_JS)),
        "/page.js" => Some(text("text/javascript; charset=utf-8", PAGE_JS)),
        "/bookmarks" => Some(html(BOOKMARKS_HTML, theme_css)),
        "/bookmarks.js" => Some(text("text/javascript; charset=utf-8", BOOKMARKS_JS)),
        "/history" => Some(html(HISTORY_HTML, theme_css)),
        "/history.js" => Some(text("text/javascript; charset=utf-8", HISTORY_JS)),
        "/downloads" => Some(html(DOWNLOADS_HTML, theme_css)),
        "/downloads.js" => Some(text("text/javascript; charset=utf-8", DOWNLOADS_JS)),
        "/settings" => Some(html(SETTINGS_HTML, theme_css)),
        "/settings.js" => Some(text("text/javascript; charset=utf-8", SETTINGS_JS)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::DARK;

    #[test]
    fn chrome_html_gets_theme_and_no_placeholder() {
        let a = chrome_asset("/index.html", &DARK.css_vars()).unwrap();
        let s = String::from_utf8(a.body).unwrap();
        assert!(s.contains("--bg-start:#0a1220"));
        assert!(!s.contains(THEME_PLACEHOLDER));
        assert!(a.mime.starts_with("text/html"));
    }

    #[test]
    fn chrome_assets_resolve_and_unknown_is_none() {
        assert!(chrome_asset("/ui.css", "").is_some());
        assert!(chrome_asset("/ui.js", "").is_some());
        assert!(chrome_asset("/secret.txt", "").is_none());
        assert!(chrome_asset("/../Cargo.toml", "").is_none());
    }

    #[test]
    fn newtab_page_assets() {
        let a = page_asset("/newtab", &DARK.css_vars()).unwrap();
        assert!(String::from_utf8(a.body)
            .unwrap()
            .contains("--accent:#1665d8"));
        assert!(page_asset("/newtab.js", "").is_some());
        assert!(page_asset("/ui.js", "").is_none());
    }

    #[test]
    fn pages_have_strict_csp_and_no_inline_script() {
        for src in [
            UI_HTML,
            NEWTAB_HTML,
            PERF_HTML,
            BOOKMARKS_HTML,
            HISTORY_HTML,
            DOWNLOADS_HTML,
            SETTINGS_HTML,
        ] {
            assert!(src.contains("Content-Security-Policy"));
            assert!(src.contains("script-src 'self'"));
            assert!(!src.contains("<script>"), "inline-скрипты запрещены CSP");
        }
    }

    #[test]
    fn performance_page_is_served_with_theme() {
        let a = page_asset("/performance", &DARK.css_vars()).unwrap();
        let s = String::from_utf8(a.body).unwrap();
        assert!(s.contains("--accent:#1665d8") && !s.contains(THEME_PLACEHOLDER));
        assert!(page_asset("/performance.js", "").is_some());
    }

    #[test]
    fn every_data_page_is_served_with_theme_and_shared_css() {
        for (path, js) in [
            ("bookmarks", "/bookmarks.js"),
            ("history", "/history.js"),
            ("downloads", "/downloads.js"),
            ("settings", "/settings.js"),
        ] {
            let a = page_asset(&format!("/{path}"), &DARK.css_vars()).unwrap();
            let s = String::from_utf8(a.body).unwrap();
            assert!(s.contains("--accent:#1665d8"), "{path}: тема");
            assert!(s.contains(".card {"), "{path}: общий CSS");
            assert!(
                !s.contains(THEME_PLACEHOLDER) && !s.contains(PAGECSS_PLACEHOLDER),
                "{path}: заглушки заменены"
            );
            assert!(page_asset(js, "").is_some(), "{path}: скрипт");
        }
        assert!(page_asset("/page.js", "").is_some());
    }

    #[test]
    fn page_scripts_never_inject_html() {
        // Названия вкладок и процессов приходят извне — только textContent.
        for js in [
            PERF_JS,
            NEWTAB_JS,
            PAGE_JS,
            BOOKMARKS_JS,
            HISTORY_JS,
            DOWNLOADS_JS,
            SETTINGS_JS,
        ] {
            assert!(
                !js.contains("innerHTML")
                    && !js.contains("insertAdjacentHTML")
                    && !js.contains("document.write")
            );
        }
        assert!(PERF_JS.contains("textContent"));
    }

    #[test]
    fn ui_script_never_uses_innerhtml_with_page_data() {
        // Заголовки вкладок приходят от сайтов — их можно выводить только через textContent.
        assert!(UI_JS.contains("textContent"));
        assert!(!UI_JS.contains("insertAdjacentHTML"));
    }
}
