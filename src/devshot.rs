//! 개발용 스크린샷 (`--features screenshot`에서만 빌드됨)
//!
//! GGL_SHOT=out.png  [GGL_SELECT=행번호] [GGL_SEARCH=검색어] [GGL_THEME=light|dark] [GGL_WAIT=초] [GGL_DIFF=파일번호]
//! [GGL_FETCH=1] 리모트 새로고침을 누른 뒤 찍는다
//! [GGL_PUSH=1] 현재 브랜치 푸시를 눌러 확인 창을 띄운 채 찍는다 (실행하지 않음)
//! [GGL_MENU=행번호] 그 행을 오른쪽 클릭해서 메뉴를 띄운 채 찍는다
//! [GGL_SCROLL=행수] 처음부터 그 행까지 스크롤하며 글자를 그려본다 (오래 켜둘 때 메모리 확인용)
//! [GGL_FILES=1] 파일 트리를 켠다  [GGL_EXPAND=폴더,폴더] 그 폴더를 펼친다  [GGL_TREE_SEARCH=검색어]
//! [GGL_FILE=경로] 트리에서 그 파일을 연다  [GGL_LINE=줄] 그 줄로 연다
//! [GGL_FIND=검색어] ⌘⇧F 내용 검색  [GGL_EDIT=1] 연 파일을 편집 모드로  [GGL_TYPE=글자] 편집기 맨 앞에 입력 (저장하지 않음)
//! [GGL_LEAVE=1] 그다음 파일을 닫으려 해서 "저장하지 않은 변경" 확인 창을 띄운다
//! [GGL_QUICK=검색어] ⌘P 빠른 열기를 띄우고 입력한다  [GGL_QUICK_SEL=번호] 그 결과를 고른다
//! 데이터(와 상세)가 다 불러와지면 화면을 저장하고 종료한다.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui;

use crate::app::App;

static STAGE: AtomicU32 = AtomicU32::new(0);
/// GGL_MENU 행의 화면 위치 (표가 그릴 때 알려준다)
static MENU_ROW: Mutex<Option<egui::Pos2>> = Mutex::new(None);
/// 다음 프레임에 넣을 가짜 입력
static EVENTS: Mutex<Vec<egui::Event>> = Mutex::new(Vec::new());
static MENU_OPENED: AtomicU32 = AtomicU32::new(0);

/// 표가 행을 그릴 때 부른다. GGL_MENU 행이면 위치를 기억한다.
pub fn row_rect(row: usize, rect: egui::Rect) {
    if std::env::var("GGL_MENU").ok().and_then(|s| s.parse().ok()) == Some(row) {
        *MENU_ROW.lock().unwrap() = Some(rect.left_center() + egui::vec2(rect.width() * 0.3, 0.0));
    }
}

/// 쌓아둔 가짜 입력을 이번 프레임 입력에 넣는다.
pub fn inject(raw: &mut egui::RawInput) {
    raw.events.append(&mut EVENTS.lock().unwrap());
}

/// GGL_MENU가 있으면 그 행을 오른쪽 클릭한다. 메뉴가 열렸으면(또는 필요 없으면) true.
fn menu_opened() -> bool {
    if std::env::var("GGL_MENU").is_err() || MENU_OPENED.load(Ordering::Relaxed) > 0 {
        return true;
    }
    let Some(pos) = *MENU_ROW.lock().unwrap() else { return false };
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    EVENTS.lock().unwrap().extend([egui::Event::PointerMoved(pos), button(true), button(false)]);
    MENU_OPENED.store(1, Ordering::Relaxed);
    false
}

pub fn tick(app: &mut App, ui: &egui::Ui) {
    let Ok(path) = std::env::var("GGL_SHOT") else { return };
    let ctx = ui.ctx();
    ctx.request_repaint();

    let shot = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    if let Some(img) = shot {
        let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        let [w, h] = img.size;
        image::save_buffer(&path, &bytes, w as u32, h as u32, image::ColorType::Rgba8).unwrap();
        let [aw, ah] = ctx.fonts(|f| f.font_image_size());
        eprintln!("font atlas: {aw}x{ah} ({:.1} MB)", (aw * ah * 4) as f64 / 1e6);
        std::process::exit(0);
    }

    let stage = STAGE.load(Ordering::Relaxed);
    match stage {
        0 if app.data.is_some() && !app.loading => {
            match std::env::var("GGL_THEME").as_deref() {
                Ok("light") => ctx.set_theme(egui::ThemePreference::Light),
                Ok("dark") => ctx.set_theme(egui::ThemePreference::Dark),
                _ => {}
            }
            if std::env::var("GGL_FILES").is_ok() && !app.settings.show_files {
                app.toggle_files();
            }
            if let Ok(dirs) = std::env::var("GGL_EXPAND") {
                app.tree_open.extend(dirs.split(',').map(str::to_string));
            }
            if let Ok(q) = std::env::var("GGL_TREE_SEARCH") {
                app.tree_filter = q;
            }
            if let Ok(q) = std::env::var("GGL_FIND") {
                app.open_find();
                app.find.query = q;
                app.find_changed();
                app.run_find();
            }
            if let Ok(q) = std::env::var("GGL_SEARCH") {
                app.search = q;
                app.update_matches();
            }
            if std::env::var("GGL_FETCH").is_ok() {
                app.fetch();
            }
            if std::env::var("GGL_PUSH").is_ok() {
                let snap = &app.data.as_ref().unwrap().snap;
                let item = snap.head_branch.as_deref().and_then(|b| crate::ops::pushes(snap, b).into_iter().next());
                if let Some(item) = item {
                    app.request(item);
                }
            }
            if let Some(row) = std::env::var("GGL_SELECT").ok().and_then(|s| s.parse().ok()) {
                app.select_row(row);
            }
            if let Ok(text) = std::env::var("GGL_QUICK") {
                app.toggle_quick();
                if let Some(q) = app.quick.as_mut() {
                    q.query = text;
                    q.sel = std::env::var("GGL_QUICK_SEL").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
                }
                app.update_quick();
            }
            STAGE.store(1, Ordering::Relaxed);
        }
        1 if app.fetch_state == crate::app::FetchState::Running || app.loading => {}
        1 if (app.settings.show_files || app.quick.is_some()) && app.tree.is_none() => {}
        1 if matches!(app.tree, Some(Ok(_))) && app.lines.is_none() => {}
        1 if app.find.running => {}
        1 if (app.selected.is_none() || app.details.is_some()) && waited(ui) => {
            let file = std::env::var("GGL_FILE").ok();
            match std::env::var("GGL_DIFF").ok().and_then(|s| s.parse().ok()) {
                Some(i) if app.diff.is_none() => app.open_diff(i),
                _ if app.diff.as_ref().is_some_and(|d| d.result.is_none()) => {}
                _ if file.is_some() && app.file_view.is_none() => {
                    let path = file.unwrap_or_default();
                    let status = match &app.tree {
                        Some(Ok(t)) => t.nodes.iter().find(|n| n.path == path).and_then(|n| n.status),
                        _ => None,
                    };
                    let line = std::env::var("GGL_LINE").ok().and_then(|s| s.parse().ok());
                    app.open_file(&path, status, line);
                }
                _ if app.file_view.as_ref().is_some_and(|v| v.result.is_none()) => {}
                _ if std::env::var("GGL_EDIT").is_ok() && app.editor.is_none() => {
                    app.start_edit();
                    if let (Some(ed), Ok(text)) = (app.editor.as_mut(), std::env::var("GGL_TYPE")) {
                        ed.text.insert_str(0, &text);
                    }
                    if std::env::var("GGL_LEAVE").is_ok() {
                        app.leave(crate::app::Leave::CloseFile);
                    }
                }
                _ => STAGE.store(2, Ordering::Relaxed),
            }
        }
        2 if !scrolled(app) => {}
        2 if !menu_opened() => {}
        // 몇 프레임 더 그려서 레이아웃이 자리 잡게 한다.
        2..=5 => STAGE.store(stage + 1, Ordering::Relaxed),
        6 => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            STAGE.store(7, Ordering::Relaxed);
        }
        _ => {}
    }
}

/// GGL_WAIT초가 지났는지 (그동안 저장소를 바꿔서 자동 새로고침을 확인할 때 쓴다)
fn waited(ui: &egui::Ui) -> bool {
    let wait: f64 = std::env::var("GGL_WAIT").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    ui.input(|i| i.time) >= wait
}

/// GGL_SCROLL 행까지 한 화면씩 내려간다. 다 내려갔으면 true.
fn scrolled(app: &mut App) -> bool {
    static ROW: AtomicU32 = AtomicU32::new(0);
    let Some(target) = std::env::var("GGL_SCROLL").ok().and_then(|s| s.parse::<u32>().ok()) else {
        return true;
    };
    let loaded = app.data.as_ref().map_or(0, |d| d.snap.commits.len()) as u32;
    let row = ROW.load(Ordering::Relaxed);
    if row >= target.min(loaded) && (!app.data.as_ref().is_some_and(|d| d.snap.more) || loaded >= target) {
        return true;
    }
    let next = (row + 30).min(loaded.saturating_sub(1));
    ROW.store(next, Ordering::Relaxed);
    app.scroll_to = Some(crate::app::ScrollTo::Center(next as usize));
    if next + 40 >= loaded {
        app.load_more();
    }
    false
}
