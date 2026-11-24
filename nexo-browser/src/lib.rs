//! Ядро Nexo Browser. Всё, что не зависит от окон и WebView2, живёт здесь
//! и покрыто unit-тестами; Windows-специфичный код — в `memory::monitor` и (шаг 3) `app`.
pub mod app;
pub mod browser;
pub mod command;
pub mod downloads;
pub mod internal_pages;
pub mod memory;
pub mod permissions;
pub mod shortcuts;
pub mod storage;
pub mod ui;
pub mod util;
