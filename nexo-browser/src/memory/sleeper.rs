//! Smart Tab Sleeping: чистая функция планирования. Она ничего не делает с WebView2 —
//! только решает, какие вкладки пора приостановить или выгрузить. Исполняет план app (шаг 4/5).
//!
//! Правила:
//! * активную вкладку не трогаем никогда;
//! * вкладки со звуком, загрузкой или несохранённым вводом не трогаем (`TabInfo::is_protected`);
//! * `Suspend` — вкладка простаивает дольше `suspend_after_secs`;
//! * `Discard` — дольше `discard_after_secs`, либо живых вкладок больше `max_live_webviews`
//!   (тогда выгружаются самые давно использованные).

use super::policy::SleepPolicy;
use crate::browser::tab::{TabId, TabInfo, TabState};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepAction {
    /// WebView2 остаётся, страница замораживается (ICoreWebView2_3::TrySuspend).
    Suspend(TabId),
    /// WebView2 уничтожается, в памяти остаются только URL, заголовок и состояние форм.
    Discard(TabId),
}

pub fn plan(
    tabs: &[TabInfo],
    active: Option<TabId>,
    policy: &SleepPolicy,
    now_ms: u64,
) -> Vec<SleepAction> {
    let idle_secs = |t: &TabInfo| now_ms.saturating_sub(t.last_active_ms) / 1000;
    let candidate = |t: &TabInfo| Some(t.id) != active && !t.is_protected() && t.is_live();

    let mut discards: Vec<TabId> = Vec::new();
    let mut suspends: Vec<TabId> = Vec::new();

    for t in tabs.iter().filter(|t| candidate(t)) {
        let idle = idle_secs(t);
        if idle >= policy.discard_after_secs {
            discards.push(t.id);
        } else if t.state == TabState::Active && idle >= policy.suspend_after_secs {
            suspends.push(t.id);
        }
    }

    // Лимит живых WebView2: выгружаем самые давно использованные.
    if policy.max_live_webviews > 0 {
        let live = tabs.iter().filter(|t| t.is_live()).count();
        let live_after = live - discards.len();
        if live_after > policy.max_live_webviews {
            let already: HashSet<TabId> = discards.iter().copied().collect();
            let mut extra: Vec<&TabInfo> = tabs
                .iter()
                .filter(|t| candidate(t) && !already.contains(&t.id))
                .collect();
            extra.sort_by_key(|t| t.last_active_ms);
            for t in extra
                .into_iter()
                .take(live_after - policy.max_live_webviews)
            {
                discards.push(t.id);
                suspends.retain(|s| *s != t.id);
            }
        }
    }

    suspends
        .into_iter()
        .map(SleepAction::Suspend)
        .chain(discards.into_iter().map(SleepAction::Discard))
        .collect()
}

/// Кнопка «Освободить память»: выгрузить всё, что разрешено правилами защиты.
pub fn plan_free_memory(tabs: &[TabInfo], active: Option<TabId>, now_ms: u64) -> Vec<SleepAction> {
    let p = SleepPolicy {
        suspend_after_secs: 0,
        discard_after_secs: 0,
        max_live_webviews: 0,
    };
    plan(tabs, active, &p, now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::policy::PerformanceMode;

    fn tab(id: u64, last_active_ms: u64) -> TabInfo {
        TabInfo::new(
            TabId(id),
            format!("https://t{id}.com/"),
            false,
            last_active_ms,
        )
    }

    const POLICY: SleepPolicy = SleepPolicy {
        suspend_after_secs: 60,
        discard_after_secs: 600,
        max_live_webviews: 0,
    };

    #[test]
    fn active_tab_is_never_touched() {
        let tabs = vec![tab(1, 0)];
        assert!(plan(&tabs, Some(TabId(1)), &POLICY, 10_000_000).is_empty());
    }

    #[test]
    fn idle_tabs_suspend_then_discard() {
        let tabs = vec![tab(1, 0), tab(2, 0)];
        let a = plan(&tabs, Some(TabId(1)), &POLICY, 120_000);
        assert_eq!(a, vec![SleepAction::Suspend(TabId(2))]);
        let b = plan(&tabs, Some(TabId(1)), &POLICY, 700_000);
        assert_eq!(b, vec![SleepAction::Discard(TabId(2))]);
    }

    #[test]
    fn recently_used_tab_is_left_alone() {
        let tabs = vec![tab(1, 0), tab(2, 100_000)];
        assert!(plan(&tabs, Some(TabId(1)), &POLICY, 120_000).is_empty());
    }

    #[test]
    fn suspended_tab_is_not_suspended_twice() {
        let mut t = tab(2, 0);
        t.state = TabState::Suspended;
        let a = plan(&[tab(1, 0), t], Some(TabId(1)), &POLICY, 120_000);
        assert!(a.is_empty());
    }

    #[test]
    fn protected_tabs_are_never_slept() {
        let mut audio = tab(2, 0);
        audio.playing_audio = true;
        let mut dl = tab(3, 0);
        dl.downloading = true;
        let mut form = tab(4, 0);
        form.has_unsaved_input = true;
        let tabs = vec![tab(1, 0), audio, dl, form];
        assert!(plan(&tabs, Some(TabId(1)), &POLICY, 99_999_999).is_empty());
        assert!(plan_free_memory(&tabs, Some(TabId(1)), 99_999_999).is_empty());
    }

    #[test]
    fn loading_tab_is_protected_until_it_finishes() {
        let mut loading = tab(2, 0);
        loading.loading = true;
        let tabs = vec![tab(1, 0), loading];
        assert!(plan(&tabs, Some(TabId(1)), &POLICY, 99_999_999).is_empty());
        let mut done = tabs.clone();
        done[1].loading = false;
        assert_eq!(
            plan(&done, Some(TabId(1)), &POLICY, 99_999_999),
            vec![SleepAction::Discard(TabId(2))]
        );
    }

    #[test]
    fn protection_reasons_are_reported_in_priority_order() {
        let mut t = tab(1, 0);
        assert_eq!(t.protection_reason(), None);
        t.loading = true;
        assert_eq!(t.protection_reason(), Some("loading"));
        t.has_unsaved_input = true;
        assert_eq!(t.protection_reason(), Some("input"));
        t.downloading = true;
        assert_eq!(t.protection_reason(), Some("downloading"));
        t.playing_audio = true;
        assert_eq!(t.protection_reason(), Some("audio"));
    }

    #[test]
    fn discarded_tabs_are_ignored() {
        let mut t = tab(2, 0);
        t.state = TabState::Discarded;
        assert!(plan(&[tab(1, 0), t], Some(TabId(1)), &POLICY, 99_999_999).is_empty());
    }

    #[test]
    fn live_limit_discards_oldest_first() {
        let p = SleepPolicy {
            suspend_after_secs: 9999,
            discard_after_secs: 9999,
            max_live_webviews: 2,
        };
        let tabs = vec![tab(1, 500), tab(2, 100), tab(3, 300), tab(4, 200)];
        let a = plan(&tabs, Some(TabId(1)), &p, 1_000);
        assert_eq!(
            a,
            vec![
                SleepAction::Discard(TabId(2)),
                SleepAction::Discard(TabId(4))
            ]
        );
    }

    #[test]
    fn live_limit_counts_already_planned_discards() {
        let p = SleepPolicy {
            suspend_after_secs: 10,
            discard_after_secs: 100,
            max_live_webviews: 3,
        };
        let tabs = vec![tab(1, 0), tab(2, 0), tab(3, 0), tab(4, 0)];
        let a = plan(&tabs, Some(TabId(1)), &p, 200_000);
        // Три вкладки старше порога выгрузки; лимит уже соблюдён, лишних выгрузок нет.
        assert_eq!(a.len(), 3);
        assert!(a.iter().all(|x| matches!(x, SleepAction::Discard(_))));
    }

    #[test]
    fn free_memory_discards_everything_allowed() {
        let tabs = vec![tab(1, 1_000), tab(2, 1_000), tab(3, 1_000)];
        let a = plan_free_memory(&tabs, Some(TabId(1)), 1_000);
        assert_eq!(
            a,
            vec![
                SleepAction::Discard(TabId(2)),
                SleepAction::Discard(TabId(3))
            ]
        );
    }

    #[test]
    fn modes_are_ordered_by_aggressiveness() {
        let s = PerformanceMode::MemorySaver.policy();
        let b = PerformanceMode::Balanced.policy();
        let m = PerformanceMode::MaxPerformance.policy();
        assert!(s.suspend_after_secs < b.suspend_after_secs);
        assert!(b.suspend_after_secs < m.suspend_after_secs);
        assert!(s.discard_after_secs < b.discard_after_secs);
        assert!(b.discard_after_secs < m.discard_after_secs);
        assert!(s.max_live_webviews < b.max_live_webviews);
        assert_eq!(m.max_live_webviews, 0);
    }
}
