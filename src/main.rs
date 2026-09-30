//! ggl — VS Code 확장 Git Graph처럼 커밋 그래프를 보여주는 가벼운 앱

mod app;
#[cfg(feature = "screenshot")]
mod devshot;
mod git;
mod graph;
mod ops;
mod style;
mod view;
mod watcher;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result {
    extend_path();

    // `ggl .` 처럼 폴더를 넘기면 그 저장소를 연다.
    let arg_repo = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .find(|p| !p.to_string_lossy().starts_with("-psn"))
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ggl")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([520.0, 320.0])
            // 아이콘을 비워두면 eframe이 실행 중에 기본 'e' 로고로 바꾸지 않고,
            // macOS가 .app에 들어 있는 그래프 로고(AppIcon.icns)를 그대로 쓴다.
            .with_icon(egui::IconData::default()),
        ..Default::default()
    };
    eframe::run_native(
        "ggl",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc, arg_repo)))),
    )
}

/// Finder/Dock에서 실행하면 PATH가 짧아서 Homebrew git을 못 찾을 수 있다.
fn extend_path() {
    let current = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<&str> = current.split(':').filter(|s| !s.is_empty()).collect();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        if !parts.contains(&extra) {
            parts.push(extra);
        }
    }
    // SAFETY: 다른 스레드가 생기기 전, main 맨 처음에 한 번만 호출한다.
    unsafe { std::env::set_var("PATH", parts.join(":")) };
}
