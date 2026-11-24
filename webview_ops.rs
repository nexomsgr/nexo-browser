//! Тонкие обёртки над COM-интерфейсами WebView2 для управления памятью.
//!
//! На Windows это реальные вызовы (`TrySuspend`, `Resume`, `MemoryUsageTargetLevel`,
//! `IsDocumentPlayingAudio`). На других платформах (используются только для проверки сборки)
//! функции ничего не делают и честно сообщают об этом возвращаемым значением — состояние
//! «приостановлена» там не выставляется.

use wry::WebView;

#[cfg(windows)]
mod imp {
    use super::WebView;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_3, ICoreWebView2_8};
    use webview2_com::TrySuspendCompletedHandler;
    use windows::core::{Interface, BOOL};
    use wry::{MemoryUsageLevel, WebViewExtWindows};

    /// Просит WebView2 приостановить страницу. `done(true)` придёт, только если страница реально
    /// приостановлена (WebView2 требует, чтобы она была скрыта). Возвращает `false`, если запрос
    /// не удалось даже отправить — тогда `done` не вызовется.
    pub fn suspend(view: &WebView, done: impl FnOnce(bool) + 'static) -> bool {
        let Ok(w3) = view.webview().cast::<ICoreWebView2_3>() else {
            return false;
        };
        let handler = TrySuspendCompletedHandler::create(Box::new(move |result, ok| {
            done(result.is_ok() && ok);
            Ok(())
        }));
        unsafe { w3.TrySuspend(&handler).is_ok() }
    }

    /// Возобновляет приостановленную страницу и возвращает обычный уровень памяти.
    pub fn resume(view: &WebView) {
        if let Ok(w3) = view.webview().cast::<ICoreWebView2_3>() {
            let _ = unsafe { w3.Resume() };
        }
        let _ = view.set_memory_usage_level(MemoryUsageLevel::Normal);
    }

    /// Просит WebView2 сжать (`low = true`) или вернуть обычный уровень использования памяти.
    /// Нужен Runtime 114+; на старых версиях вызов безвреден и ничего не делает.
    pub fn set_low_memory(view: &WebView, low: bool) {
        let level = if low {
            MemoryUsageLevel::Low
        } else {
            MemoryUsageLevel::Normal
        };
        let _ = view.set_memory_usage_level(level);
    }

    /// Воспроизводит ли документ звук прямо сейчас (официальное свойство WebView2).
    pub fn is_playing_audio(view: &WebView) -> bool {
        let Ok(w8) = view.webview().cast::<ICoreWebView2_8>() else {
            return false;
        };
        let mut playing = BOOL::default();
        unsafe { w8.IsDocumentPlayingAudio(&mut playing).is_ok() && playing.as_bool() }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::WebView;

    pub fn suspend(_view: &WebView, _done: impl FnOnce(bool) + 'static) -> bool {
        false
    }
    pub fn resume(_view: &WebView) {}
    pub fn set_low_memory(_view: &WebView, _low: bool) {}
    pub fn is_playing_audio(_view: &WebView) -> bool {
        false
    }
}

pub use imp::{is_playing_audio, resume, set_low_memory, suspend};
