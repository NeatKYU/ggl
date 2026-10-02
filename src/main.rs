//! ggl — VS Code 확장 Git Graph처럼 커밋 그래프를 보여주는 가벼운 앱

mod app;
#[cfg(feature = "screenshot")]
mod devshot;
mod git;
mod graph;
mod ops;
mod remote;
mod style;
mod tree;
mod view;
mod watcher;

use std::path::PathBuf;

use eframe::egui;

fn main() -> eframe::Result {
    extend_path();

    // `ggl .` 처럼 폴더를 넘기면 그 저장소를 연다. `--files`면 파일 트리를 펼친 채로 연다.
    // `--hand-off`면 창을 띄우지 않고, 같은 저장소를 연 ggl이 있을 때만 그 창에 넘긴다 (없으면 종료 코드 1).
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let show_files = flag("--files");
    let arg_repo = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p));

    // 같은 저장소를 연 창이 이미 있으면 새 창을 띄우지 않는다 (herdr 단축키로 여러 번 열어도 쌓이지 않게).
    if let Some(repo) = arg_repo.as_deref().and_then(|d| git::toplevel(d).ok()) {
        let req = if show_files { remote::Request::Files } else { remote::Request::Show };
        if remote::hand_off(&repo, req) {
            return Ok(());
        }
    }
    if flag("--hand-off") {
        std::process::exit(1);
    }

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
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, arg_repo, show_files)))),
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
