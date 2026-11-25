//! Команды браузера: их выдают горячие клавиши, меню и внутренние страницы.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    NewTab,
    NewPrivateTab,
    CloseActiveTab,
    ReopenClosedTab,
    NextTab,
    PrevTab,
    /// Индекс вкладки, начиная с нуля.
    SelectTab(usize),
    SelectLastTab,
    FocusAddressBar,
    Back,
    Forward,
    Reload,
    HardReload,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleFullscreen,
    BookmarkPage,
    ShowHistory,
    ShowBookmarks,
    ShowDownloads,
    FreeMemory,
    Print,
}
