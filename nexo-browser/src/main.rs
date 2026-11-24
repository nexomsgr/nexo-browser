//! Nexo Browser — точка входа.
#![windows_subsystem = "windows"]

fn main() {
    if let Err(e) = nexo_browser::app::run() {
        nexo_browser::app::show_fatal(&e);
        std::process::exit(1);
    }
}
