//! Разбор адресной строки: URL, поисковый запрос или внутренняя страница nexo://.

use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SearchEngine {
    #[default]
    Google,
    Bing,
    DuckDuckGo,
}

impl SearchEngine {
    pub fn name(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::Bing => "Bing",
            Self::DuckDuckGo => "DuckDuckGo",
        }
    }

    pub fn search_url(self, query: &str) -> String {
        let q: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        match self {
            Self::Google => format!("https://www.google.com/search?q={q}"),
            Self::Bing => format!("https://www.bing.com/search?q={q}"),
            Self::DuckDuckGo => format!("https://duckduckgo.com/?q={q}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InternalPage {
    NewTab,
    Performance,
    History,
    Bookmarks,
    Downloads,
    Settings,
    Passwords,
}

impl InternalPage {
    pub const ALL: [InternalPage; 7] = [
        Self::NewTab,
        Self::Performance,
        Self::History,
        Self::Bookmarks,
        Self::Downloads,
        Self::Settings,
        Self::Passwords,
    ];

    pub fn host(self) -> &'static str {
        match self {
            Self::NewTab => "newtab",
            Self::Performance => "performance",
            Self::History => "history",
            Self::Bookmarks => "bookmarks",
            Self::Downloads => "downloads",
            Self::Settings => "settings",
            Self::Passwords => "passwords",
        }
    }

    pub fn from_host(host: &str) -> Option<Self> {
        Some(match host {
            "newtab" | "" => Self::NewTab,
            "performance" => Self::Performance,
            "history" => Self::History,
            "bookmarks" => Self::Bookmarks,
            "downloads" => Self::Downloads,
            "settings" => Self::Settings,
            "passwords" => Self::Passwords,
            _ => return None,
        })
    }

    pub fn url(self) -> String {
        format!("nexo://{}", self.host())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Обычный адрес (http, https, file, about:blank).
    Web(String),
    /// Поисковый запрос, уже превращённый в URL поисковой системы.
    Search(String),
    Internal(InternalPage),
}

/// Превращает то, что ввёл пользователь, в цель навигации.
///
/// Безопасность: `javascript:`, `data:` и прочие нестандартные схемы из адресной
/// строки НЕ выполняются, а трактуются как поисковый текст.
pub fn resolve_input(input: &str, engine: SearchEngine) -> Target {
    let text = input.trim();
    if text.is_empty() {
        return Target::Internal(InternalPage::NewTab);
    }
    let lower = text.to_ascii_lowercase();

    if let Some(rest) = lower.strip_prefix("nexo://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        return match InternalPage::from_host(host) {
            Some(p) => Target::Internal(p),
            None => Target::Internal(InternalPage::NewTab),
        };
    }

    if lower == "about:blank" {
        return Target::Web("about:blank".to_string());
    }

    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file://")
    {
        return match Url::parse(text) {
            Ok(u) if u.scheme() == "file" || u.host_str().is_some() => Target::Web(u.into()),
            _ => Target::Search(engine.search_url(text)),
        };
    }

    if !text.chars().any(char::is_whitespace) && looks_like_host(text) {
        let host_only = host_part(text);
        let scheme = if is_local_or_ip(host_only) {
            "http"
        } else {
            "https"
        };
        if let Ok(u) = Url::parse(&format!("{scheme}://{text}")) {
            if u.host_str().is_some() {
                return Target::Web(u.into());
            }
        }
    }

    Target::Search(engine.search_url(text))
}

/// Часть до первого `/`, `?` или `#` (host[:port]).
fn authority(text: &str) -> &str {
    text.split(['/', '?', '#']).next().unwrap_or("")
}

/// Хост без порта.
fn host_part(text: &str) -> &str {
    let auth = authority(text);
    if auth.starts_with('[') {
        return match auth.find(']') {
            Some(i) => &auth[..=i],
            None => auth,
        };
    }
    auth.rsplit_once(':').map(|(h, _)| h).unwrap_or(auth)
}

fn port_ok(text: &str) -> bool {
    let auth = authority(text);
    let after = if auth.starts_with('[') {
        match auth.find(']') {
            Some(i) => &auth[i + 1..],
            None => return false,
        }
    } else {
        match auth.rsplit_once(':') {
            Some((_, p)) => return !p.is_empty() && p.parse::<u16>().is_ok(),
            None => return true,
        }
    };
    match after.strip_prefix(':') {
        Some(p) => !p.is_empty() && p.parse::<u16>().is_ok(),
        None => after.is_empty(),
    }
}

fn is_local_or_ip(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<Ipv4Addr>().is_ok()
        || host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .map(|h| h.parse::<Ipv6Addr>().is_ok())
            .unwrap_or(false)
}

fn looks_like_host(text: &str) -> bool {
    if !port_ok(text) {
        return false;
    }
    let host = host_part(text);
    if host.is_empty() {
        return false;
    }
    if is_local_or_ip(host) {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    let label_ok = |l: &str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_alphanumeric() || c == '-')
    };
    if !labels.iter().all(|l| label_ok(l)) {
        return false;
    }
    let tld = labels[labels.len() - 1];
    tld.starts_with("xn--") || (tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic))
}

/// Схема, под которой отдаются внутренние страницы содержимого.
pub const PAGE_SCHEME: &str = "nexo";
/// Схема, под которой отдаётся собственный интерфейс браузера (панель вкладок и адресная строка).
pub const UI_SCHEME: &str = "nexo-ui";

/// URL пользовательского протокола WebView2: на Windows это `http://<схема>.localhost/<путь>`,
/// на остальных платформах (только для проверки сборки) — `<схема>://localhost/<путь>`.
#[cfg(windows)]
pub fn custom_protocol_url(scheme: &str, path: &str) -> String {
    format!("http://{scheme}.localhost/{path}")
}

#[cfg(not(windows))]
pub fn custom_protocol_url(scheme: &str, path: &str) -> String {
    format!("{scheme}://localhost/{path}")
}

/// Реальный URL, который загружается в WebView для внутренней страницы.
pub fn internal_load_url(page: InternalPage) -> String {
    custom_protocol_url(PAGE_SCHEME, page.host())
}

/// Принадлежит ли `url` пользовательскому протоколу `scheme` (в любой из двух форм).
/// Используется для проверки источника IPC-сообщений и разрешённых переходов.
pub fn is_protocol_origin(url: &str, scheme: &str) -> bool {
    let Ok(u) = Url::parse(url) else { return false };
    (u.scheme() == "http" && u.host_str() == Some(&format!("{scheme}.localhost")))
        || (u.scheme() == scheme && matches!(u.host_str(), Some("localhost")))
}

/// Распознаёт внутреннюю страницу по загруженному URL (`http://nexo.localhost/newtab`,
/// `nexo://localhost/newtab`) и по «показываемой» форме `nexo://newtab`.
pub fn parse_load_url(url: &str) -> Option<InternalPage> {
    let u = Url::parse(url).ok()?;
    if is_protocol_origin(url, PAGE_SCHEME) {
        return InternalPage::from_host(u.path().trim_matches('/'));
    }
    if u.scheme() == PAGE_SCHEME && u.host_str() != Some("localhost") {
        return InternalPage::from_host(u.host_str().unwrap_or(""));
    }
    None
}

/// Приводит загружаемый URL к «показываемой» форме: внутренние страницы → `nexo://имя`.
pub fn to_display_url(url: &str) -> String {
    match parse_load_url(url) {
        Some(p) => p.url(),
        None => url.to_string(),
    }
}

/// Что показать в адресной строке: пустая строка для новой вкладки и about:blank.
pub fn address_text(display_url: &str) -> String {
    if display_url == "about:blank" || display_url == InternalPage::NewTab.url() {
        String::new()
    } else {
        display_url.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    Secure,
    Insecure,
    Internal,
    None,
}

/// Значок безопасности адресной строки. Проверка сертификата — на стороне WebView2: при ошибке
/// сертификата WebView2 сам показывает страницу-предупреждение, а не загружает сайт.
pub fn security_of(display_url: &str) -> Security {
    let l = display_url.to_ascii_lowercase();
    if l.starts_with("https://") {
        Security::Secure
    } else if l.starts_with("http://") {
        Security::Insecure
    } else if l.starts_with("nexo://") {
        Security::Internal
    } else {
        Security::None
    }
}

impl Target {
    /// URL, который нужно передать в `WebView::load_url`.
    pub fn load_url(&self) -> String {
        match self {
            Target::Web(u) | Target::Search(u) => u.clone(),
            Target::Internal(p) => internal_load_url(*p),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SearchEngine::*;

    fn web(s: &str) -> Target {
        Target::Web(s.to_string())
    }

    #[test]
    fn domain_gets_https() {
        assert_eq!(
            resolve_input("example.com", Google),
            web("https://example.com/")
        );
        assert_eq!(
            resolve_input("github.com/rust-lang/rust", Google),
            web("https://github.com/rust-lang/rust")
        );
    }

    #[test]
    fn localhost_and_ip_get_http() {
        assert_eq!(
            resolve_input("localhost:3000", Google),
            web("http://localhost:3000/")
        );
        assert_eq!(
            resolve_input("192.168.1.1", Google),
            web("http://192.168.1.1/")
        );
        assert_eq!(
            resolve_input("[::1]:8080", Google),
            web("http://[::1]:8080/")
        );
    }

    #[test]
    fn explicit_scheme_kept() {
        assert_eq!(
            resolve_input("http://example.com", Google),
            web("http://example.com/")
        );
        assert_eq!(
            resolve_input("  HTTPS://Example.com/A ", Google),
            web("https://example.com/A")
        );
    }

    #[test]
    fn plain_text_is_search() {
        assert_eq!(
            resolve_input("hello world", DuckDuckGo),
            Target::Search("https://duckduckgo.com/?q=hello+world".into())
        );
        assert_eq!(
            resolve_input("rust", Bing),
            Target::Search("https://www.bing.com/search?q=rust".into())
        );
        assert!(matches!(
            resolve_input("что такое rust", Google),
            Target::Search(_)
        ));
    }

    #[test]
    fn dangerous_schemes_become_search() {
        assert!(matches!(
            resolve_input("javascript:alert(1)", Google),
            Target::Search(_)
        ));
        assert!(matches!(
            resolve_input("data:text/html,<b>x</b>", Google),
            Target::Search(_)
        ));
    }

    #[test]
    fn domain_with_space_is_search() {
        assert!(matches!(
            resolve_input("example.com cats", Google),
            Target::Search(_)
        ));
    }

    #[test]
    fn bad_port_is_search() {
        assert!(matches!(
            resolve_input("example.com:abc", Google),
            Target::Search(_)
        ));
        assert!(matches!(
            resolve_input("example.com:99999", Google),
            Target::Search(_)
        ));
    }

    #[test]
    fn internal_pages() {
        assert_eq!(
            resolve_input("nexo://performance", Google),
            Target::Internal(InternalPage::Performance)
        );
        assert_eq!(
            resolve_input("", Google),
            Target::Internal(InternalPage::NewTab)
        );
        assert_eq!(
            resolve_input("nexo://nonsense", Google),
            Target::Internal(InternalPage::NewTab)
        );
        assert_eq!(InternalPage::Settings.url(), "nexo://settings");
    }

    #[test]
    fn search_query_is_encoded() {
        assert_eq!(
            Google.search_url("a&b=c d"),
            "https://www.google.com/search?q=a%26b%3Dc+d"
        );
    }

    #[test]
    fn load_url_roundtrip_for_all_internal_pages() {
        for p in InternalPage::ALL {
            let u = internal_load_url(p);
            assert_eq!(parse_load_url(&u), Some(p), "{u}");
            assert!(is_protocol_origin(&u, PAGE_SCHEME));
            assert!(!is_protocol_origin(&u, UI_SCHEME));
            assert_eq!(to_display_url(&u), p.url());
            assert_eq!(parse_load_url(&p.url()), Some(p));
        }
    }

    #[test]
    fn origin_check_rejects_lookalikes() {
        assert!(!is_protocol_origin(
            "http://nexo.localhost.evil.com/newtab",
            "nexo"
        ));
        assert!(!is_protocol_origin(
            "https://evil.com/nexo.localhost",
            "nexo"
        ));
        assert!(!is_protocol_origin("http://evilnexo.localhost/", "nexo"));
        assert!(!is_protocol_origin("garbage", "nexo"));
        assert_eq!(parse_load_url("https://example.com/newtab"), None);
    }

    #[test]
    fn display_and_security() {
        assert_eq!(to_display_url("https://a.com/x"), "https://a.com/x");
        assert_eq!(address_text("nexo://newtab"), "");
        assert_eq!(address_text("about:blank"), "");
        assert_eq!(address_text("nexo://performance"), "nexo://performance");
        assert_eq!(security_of("https://a.com"), Security::Secure);
        assert_eq!(security_of("HTTP://a.com"), Security::Insecure);
        assert_eq!(security_of("nexo://history"), Security::Internal);
        assert_eq!(security_of("about:blank"), Security::None);
    }

    #[test]
    fn target_load_url() {
        assert_eq!(web("https://a.com/").load_url(), "https://a.com/");
        assert_eq!(
            Target::Internal(InternalPage::Settings).load_url(),
            internal_load_url(InternalPage::Settings)
        );
    }
}
