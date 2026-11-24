//! Список вкладок, порядок, активная вкладка, закрытые вкладки (Ctrl+Shift+T).
//! Здесь только логика состояния; создание/уничтожение WebView2 делает вызывающий код.

use super::tab::{TabId, TabInfo, TabState};
use std::collections::VecDeque;

const MAX_CLOSED: usize = 25;

#[derive(Debug, Clone)]
pub struct ClosedTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub zoom: f64,
    pub index: usize,
}

#[derive(Debug, Default)]
pub struct TabManager {
    tabs: Vec<TabInfo>,
    active: Option<TabId>,
    next_id: u64,
    closed: VecDeque<ClosedTab>,
}

impl TabManager {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Default::default()
        }
    }

    pub fn tabs(&self) -> &[TabInfo] {
        &self.tabs
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_id(&self) -> Option<TabId> {
        self.active
    }

    pub fn get(&self, id: TabId) -> Option<&TabInfo> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut TabInfo> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    pub fn active(&self) -> Option<&TabInfo> {
        self.active.and_then(|id| self.get(id))
    }

    pub fn closed_count(&self) -> usize {
        self.closed.len()
    }

    fn alloc_id(&mut self) -> TabId {
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let id = TabId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Количество закреплённых вкладок (они всегда идут первыми).
    fn pinned_count(&self) -> usize {
        self.tabs.iter().take_while(|t| t.pinned).count()
    }

    /// Открывает новую вкладку в конце списка.
    pub fn open(&mut self, url: String, private: bool, now_ms: u64, activate: bool) -> TabId {
        let id = self.alloc_id();
        self.tabs.push(TabInfo::new(id, url, private, now_ms));
        if activate || self.active.is_none() {
            self.activate(id, now_ms);
        }
        id
    }

    /// Делает вкладку активной. Прежняя активная вкладка начинает отсчёт бездействия с этого момента.
    /// Если вкладка была выгружена — состояние остаётся `Discarded`, вызывающий код
    /// должен пересоздать WebView2 и затем вызвать `set_state(id, Active)`.
    pub fn activate(&mut self, id: TabId, now_ms: u64) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        if let Some(prev) = self.active {
            if prev != id {
                if let Some(t) = self.get_mut(prev) {
                    t.last_active_ms = now_ms;
                }
            }
        }
        self.active = Some(id);
        if let Some(t) = self.get_mut(id) {
            t.last_active_ms = now_ms;
        }
        true
    }

    /// Закрывает вкладку. Возвращает новую активную вкладку, если закрыли активную.
    /// Приватные вкладки не попадают в список «закрытых» (их нельзя восстановить).
    pub fn close(&mut self, id: TabId, now_ms: u64) -> Option<TabId> {
        self.remove(id, now_ms, true)
    }

    /// Убирает вкладку, которую так и не удалось показать (например, не создался WebView2):
    /// в список «закрытых» она не попадает.
    pub fn forget(&mut self, id: TabId, now_ms: u64) -> Option<TabId> {
        self.remove(id, now_ms, false)
    }

    fn remove(&mut self, id: TabId, now_ms: u64, remember: bool) -> Option<TabId> {
        let idx = self.index_of(id)?;
        let tab = self.tabs.remove(idx);
        if remember && !tab.private && !tab.url.is_empty() {
            self.closed.push_back(ClosedTab {
                url: tab.url,
                title: tab.title,
                pinned: tab.pinned,
                zoom: tab.zoom,
                index: idx,
            });
            while self.closed.len() > MAX_CLOSED {
                self.closed.pop_front();
            }
        }
        if self.active == Some(id) {
            // Как в привычных браузерах: сосед справа, иначе слева.
            let next = if idx < self.tabs.len() {
                Some(self.tabs[idx].id)
            } else {
                self.tabs.last().map(|t| t.id)
            };
            self.active = None;
            if let Some(n) = next {
                self.activate(n, now_ms);
            }
            return next;
        }
        None
    }

    /// Ctrl+Shift+T: восстанавливает последнюю закрытую вкладку на прежнее место.
    pub fn reopen_closed(&mut self, now_ms: u64) -> Option<TabId> {
        let c = self.closed.pop_back()?;
        let id = self.alloc_id();
        let mut info = TabInfo::new(id, c.url, false, now_ms);
        info.title = c.title;
        info.pinned = c.pinned;
        info.zoom = c.zoom;
        let pinned = self.pinned_count();
        let idx = if info.pinned {
            c.index.min(pinned)
        } else {
            c.index.clamp(pinned, self.tabs.len())
        };
        self.tabs.insert(idx, info);
        self.activate(id, now_ms);
        Some(id)
    }

    /// Перемещение вкладки (drag & drop). Закреплённые и обычные вкладки не смешиваются.
    pub fn move_tab(&mut self, id: TabId, to_index: usize) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        let pinned = self.tabs[from].pinned;
        let pinned_count = self.pinned_count();
        let (lo, hi) = if pinned {
            (0, pinned_count - 1)
        } else {
            (pinned_count, self.tabs.len() - 1)
        };
        let to = to_index.clamp(lo, hi);
        if to == from {
            return true;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        true
    }

    pub fn set_pinned(&mut self, id: TabId, pinned: bool) -> bool {
        let Some(from) = self.index_of(id) else {
            return false;
        };
        if self.tabs[from].pinned == pinned {
            return true;
        }
        let mut tab = self.tabs.remove(from);
        tab.pinned = pinned;
        // После удаления: закреплённая идёт в конец блока закреплённых, обычная — сразу после него.
        let to = self.pinned_count();
        self.tabs.insert(to, tab);
        true
    }

    pub fn next(&self) -> Option<TabId> {
        self.step(1)
    }

    pub fn prev(&self) -> Option<TabId> {
        self.step(-1)
    }

    fn step(&self, delta: isize) -> Option<TabId> {
        let cur = self.active.and_then(|id| self.index_of(id))?;
        let n = self.tabs.len() as isize;
        let idx = (cur as isize + delta).rem_euclid(n) as usize;
        Some(self.tabs[idx].id)
    }

    pub fn nth(&self, index: usize) -> Option<TabId> {
        self.tabs.get(index).map(|t| t.id)
    }

    pub fn last(&self) -> Option<TabId> {
        self.tabs.last().map(|t| t.id)
    }

    pub fn set_state(&mut self, id: TabId, state: TabState) {
        if let Some(t) = self.get_mut(id) {
            t.state = state;
        }
    }

    /// Вкладка успешно приостановлена. Отказ (`false`), если за время запроса она стала активной,
    /// уже не «бодрствует» или оказалась под защитой (например, началось воспроизведение звука).
    pub fn mark_suspended(&mut self, id: TabId) -> bool {
        if self.active == Some(id) {
            return false;
        }
        match self.get_mut(id) {
            Some(t) if t.state == TabState::Active && !t.is_protected() => {
                t.state = TabState::Suspended;
                true
            }
            _ => false,
        }
    }

    /// Вкладка выгружена (WebView2 уничтожен). Те же условия, что и для `mark_suspended`,
    /// но выгружать можно и приостановленную вкладку.
    pub fn mark_discarded(&mut self, id: TabId) -> bool {
        if self.active == Some(id) {
            return false;
        }
        match self.get_mut(id) {
            Some(t) if t.is_live() && !t.is_protected() => {
                t.state = TabState::Discarded;
                t.loading = false;
                true
            }
            _ => false,
        }
    }

    /// Вкладка снова работает. Возвращает прежнее состояние, чтобы вызывающий код знал, что нужно
    /// сделать: `Suspended` → `Resume`, `Discarded` → пересоздать WebView2.
    pub fn wake(&mut self, id: TabId) -> Option<TabState> {
        let t = self.get_mut(id)?;
        let prev = t.state;
        t.state = TabState::Active;
        Some(prev)
    }

    /// Сколько фоновых живых вкладок нельзя усыплять (для сообщения после «Освободить память»).
    pub fn protected_background_count(&self) -> usize {
        self.tabs
            .iter()
            .filter(|t| Some(t.id) != self.active && t.is_live() && t.is_protected())
            .count()
    }

    pub fn on_navigated(&mut self, id: TabId, url: String) {
        if let Some(t) = self.get_mut(id) {
            t.url = url;
            t.title.clear();
            t.favicon_url = None;
            t.has_unsaved_input = false;
        }
    }

    pub fn set_title(&mut self, id: TabId, title: String) {
        if let Some(t) = self.get_mut(id) {
            t.title = title;
        }
    }

    /// Число вкладок с живым WebView2 и число спящих (приостановленных или выгруженных).
    pub fn counts(&self) -> (usize, usize) {
        let awake = self
            .tabs
            .iter()
            .filter(|t| t.state == TabState::Active)
            .count();
        (awake, self.tabs.len() - awake)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr3() -> (TabManager, TabId, TabId, TabId) {
        let mut m = TabManager::new();
        let a = m.open("https://a.com/".into(), false, 0, true);
        let b = m.open("https://b.com/".into(), false, 0, false);
        let c = m.open("https://c.com/".into(), false, 0, false);
        (m, a, b, c)
    }

    #[test]
    fn first_tab_becomes_active() {
        let (m, a, _, _) = mgr3();
        assert_eq!(m.active_id(), Some(a));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn closing_active_picks_right_neighbor_then_left() {
        let (mut m, a, b, c) = mgr3();
        m.activate(b, 10);
        assert_eq!(m.close(b, 20), Some(c));
        assert_eq!(m.close(c, 30), Some(a));
        assert_eq!(m.close(a, 40), None);
        assert!(m.is_empty());
        assert_eq!(m.active_id(), None);
    }

    #[test]
    fn closing_background_tab_keeps_active() {
        let (mut m, a, b, _) = mgr3();
        assert_eq!(m.close(b, 5), None);
        assert_eq!(m.active_id(), Some(a));
    }

    #[test]
    fn reopen_restores_position_and_activates() {
        let (mut m, a, b, c) = mgr3();
        m.close(b, 5);
        let r = m.reopen_closed(6).unwrap();
        assert_eq!(m.index_of(r), Some(1));
        assert_eq!(m.get(r).unwrap().url, "https://b.com/");
        assert_eq!(m.active_id(), Some(r));
        assert_eq!(m.nth(0), Some(a));
        assert_eq!(m.nth(2), Some(c));
        assert!(m.reopen_closed(7).is_none());
    }

    #[test]
    fn private_tabs_are_not_reopenable() {
        let mut m = TabManager::new();
        let p = m.open("https://secret.example/".into(), true, 0, true);
        m.close(p, 1);
        assert_eq!(m.closed_count(), 0);
        assert!(m.reopen_closed(2).is_none());
    }

    #[test]
    fn forgotten_tab_is_not_reopenable_and_active_moves_on() {
        let (mut m, a, b, _) = mgr3();
        m.activate(b, 1);
        assert!(m.forget(b, 2).is_some());
        assert_eq!(m.closed_count(), 0);
        assert_ne!(m.active_id(), Some(b));
        assert!(m.index_of(a).is_some());
    }

    #[test]
    fn closed_history_is_bounded() {
        let mut m = TabManager::new();
        for i in 0..40 {
            let id = m.open(format!("https://s{i}.com/"), false, 0, true);
            m.close(id, 1);
        }
        assert_eq!(m.closed_count(), MAX_CLOSED);
    }

    #[test]
    fn leaving_a_tab_starts_its_idle_timer() {
        let (mut m, a, b, _) = mgr3();
        m.activate(b, 5_000);
        assert_eq!(m.get(a).unwrap().last_active_ms, 5_000);
    }

    #[test]
    fn move_tab_clamps_and_respects_pins() {
        let (mut m, a, b, c) = mgr3();
        assert!(m.set_pinned(c, true)); // c уходит в начало
        assert_eq!(m.nth(0), Some(c));
        m.move_tab(a, 0); // обычная вкладка не может встать перед закреплённой
        assert_eq!(m.nth(0), Some(c));
        assert_eq!(m.nth(1), Some(a));
        m.move_tab(b, 99);
        assert_eq!(m.last(), Some(b));
    }

    #[test]
    fn next_prev_wrap_around() {
        let (mut m, a, _, c) = mgr3();
        assert_eq!(m.prev(), Some(c));
        m.activate(c, 1);
        assert_eq!(m.next(), Some(a));
    }

    #[test]
    fn counts_split_awake_and_sleeping() {
        let (mut m, _, b, c) = mgr3();
        m.set_state(b, TabState::Suspended);
        m.set_state(c, TabState::Discarded);
        assert_eq!(m.counts(), (1, 2));
    }

    #[test]
    fn suspend_and_discard_transitions() {
        let (mut m, a, b, c) = mgr3();
        assert!(!m.mark_suspended(a), "активную вкладку усыплять нельзя");
        assert!(!m.mark_discarded(a));
        assert!(m.mark_suspended(b));
        assert_eq!(m.get(b).unwrap().state, TabState::Suspended);
        assert!(!m.mark_suspended(b), "повторно приостановить нельзя");
        assert!(m.mark_discarded(b), "приостановленную можно выгрузить");
        assert!(m.mark_discarded(c));
        assert!(!m.mark_discarded(c), "уже выгружена");
        assert_eq!(m.counts(), (1, 2));
    }

    #[test]
    fn protected_tab_cannot_be_put_to_sleep_even_if_planned_earlier() {
        // План строился, когда звука не было; пока шёл запрос, на вкладке заиграло видео.
        let (mut m, _, b, c) = mgr3();
        m.get_mut(b).unwrap().playing_audio = true;
        m.get_mut(c).unwrap().has_unsaved_input = true;
        assert!(!m.mark_suspended(b) && !m.mark_discarded(b));
        assert!(!m.mark_suspended(c) && !m.mark_discarded(c));
        assert_eq!(m.protected_background_count(), 2);
        assert_eq!(m.counts(), (3, 0));
    }

    #[test]
    fn tab_that_became_active_during_request_is_not_suspended() {
        let (mut m, _, b, _) = mgr3();
        m.activate(b, 10);
        assert!(!m.mark_suspended(b));
        assert!(!m.mark_discarded(b));
    }

    #[test]
    fn wake_reports_previous_state() {
        let (mut m, _, b, c) = mgr3();
        m.mark_suspended(b);
        m.mark_discarded(c);
        assert_eq!(m.wake(b), Some(TabState::Suspended));
        assert_eq!(m.wake(c), Some(TabState::Discarded));
        assert_eq!(m.wake(b), Some(TabState::Active));
        assert_eq!(m.wake(TabId(999)), None);
        assert_eq!(m.counts(), (3, 0));
    }

    #[test]
    fn discarding_clears_loading_flag() {
        let (mut m, _, b, _) = mgr3();
        m.get_mut(b).unwrap().loading = false;
        m.mark_discarded(b);
        assert!(!m.get(b).unwrap().loading);
    }

    #[test]
    fn navigation_clears_unsaved_flag() {
        let (mut m, a, _, _) = mgr3();
        m.get_mut(a).unwrap().has_unsaved_input = true;
        m.on_navigated(a, "https://new.com/".into());
        assert!(!m.get(a).unwrap().has_unsaved_input);
    }
}
