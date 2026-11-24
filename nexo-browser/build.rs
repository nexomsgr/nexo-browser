// Ресурсы Windows (иконка, манифест, версия) будут подключены на шаге 9 (сборка release).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
}
