//! Модель менеджера загрузок. Сами байты скачивает WebView2
//! (событие `DownloadStarting` + `BytesReceivedChanged`, шаг 6) — здесь учёт состояния и прогресса.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    InProgress,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub id: u64,
    pub url: String,
    pub path: String,
    pub received: u64,
    /// `None`, если сервер не сообщил размер.
    pub total: Option<u64>,
    pub state: DownloadState,
    pub started_ms: u64,
}

impl DownloadItem {
    pub fn file_name(&self) -> &str {
        self.path.rsplit(['\\', '/']).next().unwrap_or(&self.path)
    }

    /// Доля 0.0..=1.0; `None`, если общий размер неизвестен (показываем неопределённый индикатор).
    pub fn progress(&self) -> Option<f64> {
        match (self.state, self.total) {
            (DownloadState::Completed, _) => Some(1.0),
            (_, Some(t)) if t > 0 => Some((self.received as f64 / t as f64).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Downloads {
    items: Vec<DownloadItem>,
    next_id: u64,
}

impl Downloads {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            ..Default::default()
        }
    }

    pub fn items(&self) -> &[DownloadItem] {
        &self.items
    }

    pub fn start(&mut self, url: String, path: String, total: Option<u64>, now_ms: u64) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        self.items.push(DownloadItem {
            id,
            url,
            path,
            received: 0,
            total,
            state: DownloadState::InProgress,
            started_ms: now_ms,
        });
        id
    }

    /// Запись с заранее известным номером (его назначает обработчик события WebView2).
    pub fn start_with_id(
        &mut self,
        id: u64,
        url: String,
        path: String,
        total: Option<u64>,
        now_ms: u64,
    ) {
        self.items.retain(|d| d.id != id);
        self.next_id = self.next_id.max(id + 1);
        self.items.push(DownloadItem {
            id,
            url,
            path,
            received: 0,
            total,
            state: DownloadState::InProgress,
            started_ms: now_ms,
        });
    }

    pub fn get(&self, id: u64) -> Option<&DownloadItem> {
        self.items.iter().find(|d| d.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut DownloadItem> {
        self.items.iter_mut().find(|d| d.id == id)
    }

    pub fn update(&mut self, id: u64, received: u64, total: Option<u64>) {
        if let Some(d) = self.get_mut(id) {
            if d.state == DownloadState::InProgress {
                d.received = received;
                if total.is_some() {
                    d.total = total;
                }
            }
        }
    }

    fn finish_with(&mut self, id: u64, state: DownloadState) {
        if let Some(d) = self.get_mut(id) {
            if d.state == DownloadState::InProgress {
                d.state = state;
                if state == DownloadState::Completed {
                    if let Some(t) = d.total {
                        d.received = t;
                    }
                }
            }
        }
    }

    pub fn complete(&mut self, id: u64) {
        self.finish_with(id, DownloadState::Completed)
    }

    pub fn cancel(&mut self, id: u64) {
        self.finish_with(id, DownloadState::Cancelled)
    }

    pub fn fail(&mut self, id: u64) {
        self.finish_with(id, DownloadState::Failed)
    }

    pub fn active_count(&self) -> usize {
        self.items
            .iter()
            .filter(|d| d.state == DownloadState::InProgress)
            .count()
    }

    pub fn has_active(&self) -> bool {
        self.active_count() > 0
    }

    /// Убирает из списка всё, кроме идущих загрузок.
    pub fn clear_finished(&mut self) {
        self.items.retain(|d| d.state == DownloadState::InProgress);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_and_completion() {
        let mut d = Downloads::new();
        let id = d.start(
            "https://x.com/f.zip".into(),
            r"C:\Users\me\Downloads\f.zip".into(),
            Some(200),
            0,
        );
        assert_eq!(d.items()[0].file_name(), "f.zip");
        assert_eq!(d.items()[0].progress(), Some(0.0));
        d.update(id, 50, None);
        assert_eq!(d.items()[0].progress(), Some(0.25));
        assert!(d.has_active());
        d.complete(id);
        assert_eq!(d.items()[0].progress(), Some(1.0));
        assert_eq!(d.items()[0].received, 200);
        assert!(!d.has_active());
    }

    #[test]
    fn unknown_size_has_no_percentage() {
        let mut d = Downloads::new();
        let id = d.start("u".into(), "/tmp/a.bin".into(), None, 0);
        d.update(id, 1234, None);
        assert_eq!(d.items()[0].progress(), None);
        d.update(id, 2000, Some(4000));
        assert_eq!(d.items()[0].progress(), Some(0.5));
    }

    #[test]
    fn finished_downloads_are_final() {
        let mut d = Downloads::new();
        let id = d.start("u".into(), "a".into(), Some(10), 0);
        d.cancel(id);
        d.update(id, 9, None);
        d.complete(id);
        assert_eq!(d.items()[0].state, DownloadState::Cancelled);
        assert_eq!(d.items()[0].received, 0);
    }

    #[test]
    fn clear_finished_keeps_running() {
        let mut d = Downloads::new();
        let a = d.start("a".into(), "a".into(), None, 0);
        let b = d.start("b".into(), "b".into(), None, 0);
        d.fail(a);
        d.clear_finished();
        assert_eq!(d.items().len(), 1);
        assert_eq!(d.items()[0].id, b);
        assert_eq!(d.active_count(), 1);
    }

    #[test]
    fn start_with_id_keeps_the_given_number_and_advances_the_counter() {
        let mut d = Downloads::new();
        d.start_with_id(7, "https://a/f.zip".into(), "C:\\f.zip".into(), Some(10), 0);
        assert_eq!(d.get(7).unwrap().file_name(), "f.zip");
        assert_eq!(d.start("u".into(), "p".into(), None, 0), 8);
        // Повторный запуск с тем же номером заменяет запись, а не дублирует её.
        d.start_with_id(7, "https://a/g.zip".into(), "g.zip".into(), None, 0);
        assert_eq!(d.items().len(), 2);
        assert_eq!(d.get(7).unwrap().url, "https://a/g.zip");
    }
}
